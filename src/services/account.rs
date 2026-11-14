use anyhow::Result;

use crate::console::{p_print, Color};
use crate::models::{Config, Credentials};

pub async fn delete_account(cfg: &Config, creds: &Credentials) -> Result<()> {
    let client = super::upload::get_session(cfg, creds).await?;
    client.delete_account().await?;
    p_print(&format!("Deleted MEGA account {}", creds.email), Color::Ok);
    Ok(())
}

pub async fn create_folder(cfg: &Config, creds: &Credentials, name: &str) -> Result<String> {
    let client = super::upload::get_session(cfg, creds).await?;
    let node = client.create_folder(name, None).await?;
    p_print(&format!("Created folder '{}'", name), Color::Ok);
    Ok(node.handle)
}

pub fn change_password() {
    p_print(
        "Changing a MEGA password is not supported. Create a new account instead.",
        Color::Warning,
    );
}
