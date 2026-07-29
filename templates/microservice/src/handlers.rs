//! Job handlers

use crate::jobs::{EmailJob, NotificationJob, ProcessDataJob};
use serde_json::Value;
use tracing::{debug, info};

/// Job handler registry
pub struct JobHandlers;

impl JobHandlers {
    /// Handle a job based on its type
    pub async fn handle(job_type: &str, payload: Value) -> Result<(), String> {
        match job_type {
            "send_email" => Self::handle_email(payload).await,
            "send_notification" => Self::handle_notification(payload).await,
            "process_data" => Self::handle_process_data(payload).await,
            _ => Err(format!("Unknown job type: {}", job_type)),
        }
    }

    async fn handle_email(payload: Value) -> Result<(), String> {
        let job: EmailJob = serde_json::from_value(payload).map_err(|e| e.to_string())?;

        info!(to = %job.to, subject = %job.subject, "Sending email");

        // Simulate email sending
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        debug!("Email sent successfully");
        Ok(())
    }

    async fn handle_notification(payload: Value) -> Result<(), String> {
        let job: NotificationJob = serde_json::from_value(payload).map_err(|e| e.to_string())?;

        info!(user_id = %job.user_id, channel = ?job.channel, "Sending notification");

        // Simulate notification sending
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        debug!("Notification sent successfully");
        Ok(())
    }

    async fn handle_process_data(payload: Value) -> Result<(), String> {
        let job: ProcessDataJob = serde_json::from_value(payload).map_err(|e| e.to_string())?;

        info!(data_id = %job.data_id, operation = %job.operation, "Processing data");

        // Simulate data processing
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        debug!("Data processed successfully");
        Ok(())
    }
}

