//! Command handling for CQRS

use async_trait::async_trait;
use dashmap::DashMap;
use std::any::{Any, TypeId};
use std::sync::Arc;
use thiserror::Error;

/// Command trait
///
/// Commands represent write operations in CQRS.
pub trait Command: Send + Sync + 'static {
    /// Command result type
    type Result: Send;
}

/// Command handler trait
#[async_trait]
pub trait CommandHandler<C: Command>: Send + Sync {
    /// Handle the command
    async fn handle(&self, command: C) -> Result<C::Result, CommandError>;
}

/// Command error
#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Command execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Handler not found for command")]
    HandlerNotFound,

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Business rule violation: {0}")]
    BusinessRuleViolation(String),
}

/// Type-erased command handler
#[async_trait]
trait DynCommandHandler: Send + Sync {
    async fn handle_dyn(
        &self,
        command: Box<dyn Any + Send>,
    ) -> Result<Box<dyn Any + Send>, CommandError>;
}

/// Wrapper for typed command handlers
struct TypedCommandHandler<C: Command, H: CommandHandler<C>> {
    handler: H,
    _phantom: std::marker::PhantomData<C>,
}

impl<C: Command, H: CommandHandler<C>> TypedCommandHandler<C, H> {
    fn new(handler: H) -> Self {
        Self {
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<C: Command, H: CommandHandler<C>> DynCommandHandler for TypedCommandHandler<C, H> {
    async fn handle_dyn(
        &self,
        command: Box<dyn Any + Send>,
    ) -> Result<Box<dyn Any + Send>, CommandError> {
        match command.downcast::<C>() {
            Ok(cmd) => {
                let result = self.handler.handle(*cmd).await?;
                Ok(Box::new(result))
            }
            Err(_) => Err(CommandError::ExecutionFailed("Type mismatch".to_string())),
        }
    }
}

/// Command bus
pub struct CommandBus {
    handlers: DashMap<TypeId, Arc<dyn DynCommandHandler>>,
}

impl CommandBus {
    /// Create new command bus
    pub fn new() -> Self {
        Self {
            handlers: DashMap::new(),
        }
    }

    /// Register a command handler
    pub fn register<C, H>(&self, handler: H)
    where
        C: Command,
        H: CommandHandler<C> + 'static,
    {
        let type_id = TypeId::of::<C>();
        let handler = Arc::new(TypedCommandHandler::new(handler));
        self.handlers.insert(type_id, handler);
    }

    /// Execute a command
    pub async fn execute<C>(&self, command: C) -> Result<C::Result, CommandError>
    where
        C: Command,
    {
        let type_id = TypeId::of::<C>();

        // Clone the handler out and drop the map guard before awaiting. A
        // `Ref` from `DashMap::get` holds that shard's read lock, so holding it
        // across the handler future would block any concurrent `register` that
        // lands on the same shard for the whole duration of the handler — and a
        // handler that re-entrantly calls `register` would deadlock on itself.
        let handler = {
            let entry = self
                .handlers
                .get(&type_id)
                .ok_or(CommandError::HandlerNotFound)?;
            Arc::clone(entry.value())
        };

        let boxed_command: Box<dyn Any + Send> = Box::new(command);
        let result = handler.handle_dyn(boxed_command).await?;

        match result.downcast::<C::Result>() {
            Ok(result) => Ok(*result),
            Err(_) => Err(CommandError::ExecutionFailed(
                "Result type mismatch".to_string(),
            )),
        }
    }
}

impl Default for CommandBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CreateUserCommand {
        email: String,
    }

    impl Command for CreateUserCommand {
        type Result = String; // Returns user ID
    }

    struct CreateUserHandler;

    #[async_trait]
    impl CommandHandler<CreateUserCommand> for CreateUserHandler {
        async fn handle(&self, command: CreateUserCommand) -> Result<String, CommandError> {
            Ok(format!("user-{}", command.email))
        }
    }

    #[tokio::test]
    async fn test_command_bus() {
        let bus = CommandBus::new();
        bus.register::<CreateUserCommand, _>(CreateUserHandler);

        let command = CreateUserCommand {
            email: "alice@example.com".to_string(),
        };

        let result = bus.execute(command).await.unwrap();
        assert_eq!(result, "user-alice@example.com");
    }

    #[tokio::test]
    async fn test_command_bus_handler_not_found() {
        let bus = CommandBus::new();

        let command = CreateUserCommand {
            email: "bob@example.com".to_string(),
        };

        let err = bus.execute(command).await.unwrap_err();
        assert!(matches!(err, CommandError::HandlerNotFound));
    }

    struct AnotherCommand {
        value: u32,
    }

    impl Command for AnotherCommand {
        type Result = u32;
    }

    struct AnotherCommandHandler;

    #[async_trait]
    impl CommandHandler<AnotherCommand> for AnotherCommandHandler {
        async fn handle(&self, command: AnotherCommand) -> Result<u32, CommandError> {
            Ok(command.value)
        }
    }

    #[tokio::test]
    async fn test_command_bus_independent_types_do_not_collide() {
        // Two distinct command types registered on the same bus must be
        // routed to their own handlers without triggering the downcast
        // "type mismatch" error path.
        let bus = CommandBus::new();
        bus.register::<CreateUserCommand, _>(CreateUserHandler);
        bus.register::<AnotherCommand, _>(AnotherCommandHandler);

        let user_result = bus
            .execute(CreateUserCommand {
                email: "carol@example.com".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(user_result, "user-carol@example.com");

        let another_result = bus.execute(AnotherCommand { value: 42 }).await.unwrap();
        assert_eq!(another_result, 42);
    }

    struct ReentrantRegisterHandler {
        bus: Arc<std::sync::OnceLock<Arc<CommandBus>>>,
    }

    #[async_trait]
    impl CommandHandler<CreateUserCommand> for ReentrantRegisterHandler {
        async fn handle(&self, command: CreateUserCommand) -> Result<String, CommandError> {
            // Registering under the *same* command type guarantees the same
            // DashMap shard as the entry `execute` looked up. If `execute` were
            // still holding that shard's read guard, this write would block
            // forever.
            let bus = self.bus.get().expect("bus wired before execute").clone();
            bus.register::<CreateUserCommand, _>(CreateUserHandler);
            Ok(format!("user-{}", command.email))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_handler_can_register_while_running() {
        let cell = Arc::new(std::sync::OnceLock::new());
        let bus = Arc::new(CommandBus::new());
        // The cell was just created, so this always succeeds; the error variant
        // carries the un-stored `Arc` and is not `Debug`.
        let _ = cell.set(bus.clone());

        bus.register::<CreateUserCommand, _>(ReentrantRegisterHandler { bus: cell });

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            bus.execute(CreateUserCommand {
                email: "dana@example.com".to_string(),
            }),
        )
        .await
        .expect("execute must not hold a map guard across the handler await");

        assert_eq!(result.unwrap(), "user-dana@example.com");
    }
}
