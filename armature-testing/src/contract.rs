//! Contract Testing (Pact)
//!
//! Provides consumer-driven contract testing capabilities.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

/// Contract testing errors
#[derive(Debug, Error)]
pub enum ContractError {
    #[error("Verification failed: {0}")]
    VerificationFailed(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Invalid contract: {0}")]
    InvalidContract(String),
}

/// HTTP method for contract
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ContractMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

/// Contract request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRequest {
    /// HTTP method
    pub method: ContractMethod,

    /// Request path
    pub path: String,

    /// Query parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<HashMap<String, String>>,

    /// Request headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Request body
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl ContractRequest {
    /// Create new contract request
    pub fn new(method: ContractMethod, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            query: None,
            headers: None,
            body: None,
        }
    }

    /// Add query parameter
    pub fn with_query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Add header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Set body
    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body);
        self
    }
}

/// Contract response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractResponse {
    /// HTTP status code
    pub status: u16,

    /// Response headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Response body
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl ContractResponse {
    /// Create new contract response
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: None,
            body: None,
        }
    }

    /// Add header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(HashMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Set body
    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body);
        self
    }
}

/// Contract interaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInteraction {
    /// Interaction description
    pub description: String,

    /// Provider state (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<String>,

    /// Request
    pub request: ContractRequest,

    /// Response
    pub response: ContractResponse,
}

impl ContractInteraction {
    /// Create new interaction
    pub fn new(
        description: impl Into<String>,
        request: ContractRequest,
        response: ContractResponse,
    ) -> Self {
        Self {
            description: description.into(),
            provider_state: None,
            request,
            response,
        }
    }

    /// Set provider state
    pub fn with_provider_state(mut self, state: impl Into<String>) -> Self {
        self.provider_state = Some(state.into());
        self
    }
}

/// Consumer contract (Pact)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    /// Consumer name
    pub consumer: Participant,

    /// Provider name
    pub provider: Participant,

    /// Interactions
    pub interactions: Vec<ContractInteraction>,

    /// Metadata
    pub metadata: ContractMetadata,
}

/// Participant (consumer or provider)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub name: String,
}

impl Participant {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub struct ContractMetadata {
    pub pact_specification: PactSpecification,
}

/// Pact specification version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PactSpecification {
    pub version: String,
}

impl Default for PactSpecification {
    fn default() -> Self {
        Self {
            version: "3.0.0".to_string(),
        }
    }
}

/// Contract builder
pub struct ContractBuilder {
    consumer: String,
    provider: String,
    interactions: Vec<ContractInteraction>,
}

impl ContractBuilder {
    /// Create new contract builder
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::contract::*;
    ///
    /// let builder = ContractBuilder::new("MyConsumer", "MyProvider");
    /// ```
    pub fn new(consumer: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            consumer: consumer.into(),
            provider: provider.into(),
            interactions: Vec::new(),
        }
    }

    /// Add interaction
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::contract::*;
    ///
    /// let request = ContractRequest::new(ContractMethod::Get, "/api/users/1");
    /// let response = ContractResponse::new(200)
    ///     .with_body(serde_json::json!({"id": 1, "name": "Alice"}));
    ///
    /// let mut builder = ContractBuilder::new("Frontend", "Backend");
    /// builder.add_interaction(ContractInteraction::new(
    ///     "get user by ID",
    ///     request,
    ///     response,
    /// ));
    /// ```
    pub fn add_interaction(&mut self, interaction: ContractInteraction) -> &mut Self {
        self.interactions.push(interaction);
        self
    }

    /// Build contract
    pub fn build(self) -> Contract {
        Contract {
            consumer: Participant::new(self.consumer),
            provider: Participant::new(self.provider),
            interactions: self.interactions,
            metadata: ContractMetadata::default(),
        }
    }
}

/// Contract manager
pub struct ContractManager {
    contracts_dir: PathBuf,
}

impl ContractManager {
    /// Create new contract manager
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::contract::ContractManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = ContractManager::new(PathBuf::from("./pacts"));
    /// ```
    pub fn new(contracts_dir: PathBuf) -> Self {
        Self { contracts_dir }
    }

    /// Save contract to file
    pub fn save(&self, contract: &Contract) -> Result<(), ContractError> {
        // Create contracts directory if it doesn't exist
        std::fs::create_dir_all(&self.contracts_dir)
            .map_err(|e| ContractError::IoError(e.to_string()))?;

        // Generate filename
        let filename = format!(
            "{}-{}.json",
            contract.consumer.name.to_lowercase(),
            contract.provider.name.to_lowercase()
        );
        let path = self.contracts_dir.join(filename);

        // Serialize contract
        let json = serde_json::to_string_pretty(contract)
            .map_err(|e| ContractError::SerializationError(e.to_string()))?;

        // Write to file
        std::fs::write(path, json).map_err(|e| ContractError::IoError(e.to_string()))?;

        Ok(())
    }

    /// Load contract from file
    pub fn load(&self, consumer: &str, provider: &str) -> Result<Contract, ContractError> {
        let filename = format!(
            "{}-{}.json",
            consumer.to_lowercase(),
            provider.to_lowercase()
        );
        let path = self.contracts_dir.join(filename);

        let content =
            std::fs::read_to_string(path).map_err(|e| ContractError::IoError(e.to_string()))?;

        let contract: Contract = serde_json::from_str(&content)
            .map_err(|e| ContractError::SerializationError(e.to_string()))?;

        Ok(contract)
    }

    /// List all contracts
    pub fn list(&self) -> Result<Vec<(String, String)>, ContractError> {
        let entries = std::fs::read_dir(&self.contracts_dir)
            .map_err(|e| ContractError::IoError(e.to_string()))?;

        let mut contracts = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| ContractError::IoError(e.to_string()))?;
            let filename = entry.file_name().to_string_lossy().to_string();

            if filename.ends_with(".json")
                && let Some(parts) = filename.strip_suffix(".json")
            {
                let parts: Vec<&str> = parts.split('-').collect();
                if parts.len() >= 2 {
                    contracts.push((parts[0].to_string(), parts[1].to_string()));
                }
            }
        }

        Ok(contracts)
    }
}

/// Contract verifier
pub struct ContractVerifier;

impl ContractVerifier {
    /// Verify a contract interaction
    ///
    /// This is a simplified verification - in a real implementation,
    /// you would make actual HTTP requests and compare responses.
    pub fn verify_interaction(
        interaction: &ContractInteraction,
        actual_response: &ContractResponse,
    ) -> Result<(), ContractError> {
        // Verify status
        if interaction.response.status != actual_response.status {
            return Err(ContractError::VerificationFailed(format!(
                "Status mismatch: expected {}, got {}",
                interaction.response.status, actual_response.status
            )));
        }

        // Verify headers (if specified)
        if let Some(expected_headers) = &interaction.response.headers {
            let actual_headers = actual_response.headers.as_ref().ok_or_else(|| {
                ContractError::VerificationFailed("Missing response headers".to_string())
            })?;

            for (key, expected_value) in expected_headers {
                let actual_value = actual_headers.get(key).ok_or_else(|| {
                    ContractError::VerificationFailed(format!("Missing header: {}", key))
                })?;

                if actual_value != expected_value {
                    return Err(ContractError::VerificationFailed(format!(
                        "Header mismatch for '{}': expected '{}', got '{}'",
                        key, expected_value, actual_value
                    )));
                }
            }
        }

        // Verify body (if specified)
        if let Some(expected_body) = &interaction.response.body {
            let actual_body = actual_response.body.as_ref().ok_or_else(|| {
                ContractError::VerificationFailed("Missing response body".to_string())
            })?;

            if expected_body != actual_body {
                return Err(ContractError::VerificationFailed(format!(
                    "Body mismatch: expected {:?}, got {:?}",
                    expected_body, actual_body
                )));
            }
        }

        Ok(())
    }

    /// Verify entire contract
    pub fn verify_contract(
        _contract: &Contract,
        interactions_results: &[(ContractInteraction, ContractResponse)],
    ) -> Result<(), ContractError> {
        for (interaction, actual_response) in interactions_results {
            Self::verify_interaction(interaction, actual_response)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_builder() {
        let request = ContractRequest::new(ContractMethod::Get, "/api/users/1");
        let response =
            ContractResponse::new(200).with_body(serde_json::json!({"id": 1, "name": "Alice"}));

        let mut builder = ContractBuilder::new("Frontend", "Backend");
        builder.add_interaction(ContractInteraction::new(
            "get user by ID",
            request,
            response,
        ));

        let contract = builder.build();
        assert_eq!(contract.consumer.name, "Frontend");
        assert_eq!(contract.provider.name, "Backend");
        assert_eq!(contract.interactions.len(), 1);
    }

    #[test]
    fn test_contract_verification_success() {
        let interaction = ContractInteraction::new(
            "get user",
            ContractRequest::new(ContractMethod::Get, "/api/users/1"),
            ContractResponse::new(200).with_body(serde_json::json!({"id": 1})),
        );

        let actual = ContractResponse::new(200).with_body(serde_json::json!({"id": 1}));

        assert!(ContractVerifier::verify_interaction(&interaction, &actual).is_ok());
    }

    #[test]
    fn test_contract_verification_status_mismatch() {
        let interaction = ContractInteraction::new(
            "get user",
            ContractRequest::new(ContractMethod::Get, "/api/users/1"),
            ContractResponse::new(200),
        );

        let actual = ContractResponse::new(404);

        assert!(ContractVerifier::verify_interaction(&interaction, &actual).is_err());
    }

    #[test]
    fn test_contract_request_builder() {
        let request = ContractRequest::new(ContractMethod::Post, "/api/users")
            .with_query("page", "1")
            .with_header("Content-Type", "application/json")
            .with_body(serde_json::json!({"name": "Bob"}));

        assert_eq!(request.method, ContractMethod::Post);
        assert_eq!(request.path, "/api/users");
        assert!(request.query.is_some());
        assert!(request.headers.is_some());
        assert!(request.body.is_some());
    }
}
