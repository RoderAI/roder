//! Freshness diagnostics name changed fields without recording their values.
use super::*;
impl Page {
    pub(crate) async fn fresh(
        &mut self,
        observation: &Value,
        action: Option<&Value>,
    ) -> anyhow::Result<bool> {
        Ok(self
            .freshness_changes(observation, action)
            .await?
            .is_empty())
    }
    pub(crate) async fn freshness_changes(
        &mut self,
        observation: &Value,
        action: Option<&Value>,
    ) -> anyhow::Result<Vec<String>> {
        if let Some(action) = action
            && matches!(action["kind"].as_str(), Some("click") | Some("select"))
        {
            let Some(node) = action["node"].as_i64() else {
                return Ok(vec!["target_missing".into()]);
            };
            let current = self.evaluate(&format!("(() => {{ const c=window.__jevFast; return c?.pageKey ? [c.pageKey(),c.guard(c.nodes.get({node}))] : null; }})()")).await?;
            let mut changed = changes(
                &observation["page_key"],
                &current[0],
                &[
                    "document",
                    "url",
                    "scroll_x",
                    "scroll_y",
                    "viewport_width",
                    "viewport_height",
                    "form_state",
                ],
            );
            changed.extend(changes(
                &observation["guards"][node.to_string()],
                &current[1],
                &[
                    "target_identity",
                    "target_role",
                    "target_label",
                    "target_value",
                    "target_checked",
                    "target_option",
                    "target_readonly",
                    "target_disabled",
                    "target_aria_disabled",
                    "target_expanded",
                    "target_aria_checked",
                    "target_selected",
                    "target_destination",
                    "target_form",
                    "target_context",
                ],
            ));
            return Ok(changed);
        }
        let marker = self.read(super::snapshot::Read::Marker).await?;
        Ok(changes(
            &observation["marker"],
            &marker,
            &[
                "document",
                "url",
                "scroll_x",
                "scroll_y",
                "viewport_width",
                "viewport_height",
                "title",
                "page_text",
                "disabled_controls",
                "controls",
                "form_state",
            ],
        ))
    }
}
fn changes(before: &Value, after: &Value, names: &[&str]) -> Vec<String> {
    if before == after {
        return Vec::new();
    }
    if !before.is_array() || !after.is_array() {
        return vec!["observation_unavailable".into()];
    }
    names
        .iter()
        .enumerate()
        .filter(|(i, _)| before[*i] != after[*i])
        .map(|(_, name)| (*name).into())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_name_fields_without_echoing_values() {
        let names = ["label", "destination"];
        let before = json!(["private label", "https://private.test"]);
        assert_eq!(
            changes(&before, &json!(["private label", "changed secret"]), &names),
            vec!["destination"]
        );
        assert!(changes(&before, &before, &names).is_empty());
        assert_eq!(
            changes(&before, &Value::Null, &names),
            vec!["observation_unavailable"]
        );
    }
}
