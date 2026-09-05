//! Scoped, bounded directory metadata requests. Payloads still use tus / HTTP.
use super::*;
use crate::pairing::PairingClient;
use reqwest::Method;

pub struct DirectoryClient {
    pairing: PairingClient,
}
impl DirectoryClient {
    pub fn new(base: &str, token: &str, insecure: bool, role: &str) -> Result<Self> {
        let pairing = PairingClient::new(base, token, insecure)?;
        let handshake = pairing.handshake()?;
        ensure!(
            handshake.role == role && handshake.state == "ready",
            "Confirm the paired Folder using the correct device credential first."
        );
        Ok(Self { pairing })
    }
    pub fn state(&self) -> Result<DirectoryState> {
        let state: DirectoryState =
            self.pairing
                .request_limited(Method::GET, "api/v1/directory", None::<&()>, MAX_BODY)?;
        state.validate()?;
        Ok(state)
    }
    pub fn publish(&self, update: &InventoryUpdate) -> Result<Inventory> {
        update.inventory.validate()?;
        let inventory: Inventory = self.pairing.request_limited(
            Method::PUT,
            "api/v1/directory/index",
            Some(update),
            MAX_BODY,
        )?;
        ensure!(
            inventory == update.inventory,
            "Receiver index was not confirmed."
        );
        Ok(inventory)
    }
    pub fn acknowledge(&self, receipt: &DirectoryAck) -> Result<()> {
        let result: serde_json::Value = self.pairing.request_limited(
            Method::POST,
            "api/v1/directory/ack",
            Some(receipt),
            16 * 1024,
        )?;
        ensure!(
            result.get("acknowledged").and_then(|v| v.as_bool()) == Some(true),
            "Directory receipt was not confirmed."
        );
        Ok(())
    }
}
