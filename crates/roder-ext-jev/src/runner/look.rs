//! Reading the session's tab once more, as Jev reads it, after another
//! driver moved it: the result then shows the page the call ended on in the
//! same shape as ever (its text, headings, the frames Jev reads, and the
//! options Jev could act on), not only what the fallback's last look saw.
//! No decision is made and nothing is pressed.

use std::time::Duration;

use serde_json::json;

use crate::cdp::Connection;
use crate::engine::JevBrowser;
use crate::fallback::FinalPage;
use crate::page::Page;
use crate::secret::Secrets;
use crate::session::SessionTabs;
use crate::space::action_space;

/// The longest the read may take.
const LOOK_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn look_again(
    endpoint: &str,
    tabs: &SessionTabs,
    secrets: &Secrets,
    autoconsent: bool,
) -> Option<FinalPage> {
    tokio::time::timeout(LOOK_TIMEOUT, read(endpoint, tabs, secrets, autoconsent))
        .await
        .ok()?
        .ok()
}

async fn read(
    endpoint: &str,
    tabs: &SessionTabs,
    secrets: &Secrets,
    autoconsent: bool,
) -> anyhow::Result<FinalPage> {
    let connection = Connection::connect(endpoint).await?;
    // Not brought forward: the tab is where the fallback left it.
    let (mut page, _gone) = Page::resume(connection, &tabs.owned(), false, autoconsent).await?;
    let mut observation = page.observe().await?;
    secrets.scrub_observation(&mut observation);
    let facts = JevBrowser::describe(&mut page).await.ok().flatten();
    let mut facts = facts.unwrap_or_default();
    facts.headings = facts
        .headings
        .iter()
        .map(|heading| secrets.scrub(heading))
        .collect();
    facts.frames = serde_json::from_value(observation["frames"].clone()).unwrap_or_default();
    let text = observation["text"].as_str().unwrap_or_default();
    let observed = action_space(
        observation["actions"]
            .as_array()
            .map_or(&[][..], Vec::as_slice),
    );
    Ok(FinalPage {
        url: observation["url"].as_str().unwrap_or_default().to_string(),
        title: observation["title"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        visible_text: text.chars().take(6000).collect(),
        controls: json!(crate::agent::controls(&observation)),
        page: serde_json::to_value(facts).unwrap_or_default(),
        observed_elements: observed.elements.len(),
    })
}
