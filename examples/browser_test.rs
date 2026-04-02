//! Quick smoke test: navigate to Google, search for "opencrabs".

use opencrabs::brain::tools::browser::BrowserManager;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mgr = Arc::new(BrowserManager::with_headless(false));

    println!("[1/3] Navigating to Google...");
    let page = mgr.get_or_create_page(None).await?;
    page.goto("https://www.google.com").await?;
    let _ = page.wait_for_navigation().await;

    let title = page.get_title().await?.unwrap_or_default();
    println!("  -> Title: {title}");

    println!("[2/3] Typing 'opencrabs' into search box...");
    let search_box = page
        .find_element("textarea[name='q'], input[name='q']")
        .await?;
    search_box.click().await?;
    search_box.type_str("opencrabs").await?;

    println!("[3/3] Pressing Enter...");
    search_box.press_key("Enter").await?;
    let _ = page.wait_for_navigation().await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let results_title = page.get_title().await?.unwrap_or_default();
    let results_url = page.url().await?.unwrap_or_default();
    println!("  -> Title: {results_title}");
    println!("  -> URL: {results_url}");

    println!("\nBrowser test complete! Chrome window should be visible.");
    println!("Press Ctrl+C to exit...");

    // Keep alive so user can see the headed browser
    tokio::signal::ctrl_c().await?;
    mgr.shutdown().await;
    Ok(())
}
