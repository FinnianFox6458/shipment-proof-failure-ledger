use shipment_failure_ledger::{process_event, sample_delivered_event, AgentDecision, InfraiClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = InfraiClient::from_env()?;
    let event = sample_delivered_event();
    let decision = process_event(&client, event, None).await?;

    match decision {
        AgentDecision::HoldAndCapture { reason } => {
            println!("shipment held; exception captured: {reason}");
        }
        AgentDecision::Complete { .. } => println!("shipment completed with proof"),
        AgentDecision::Continue => println!("shipment remains in transit"),
    }
    Ok(())
}
