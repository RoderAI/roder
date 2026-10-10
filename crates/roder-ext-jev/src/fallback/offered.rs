//! The tools the fallback model is offered: Roder's direct browser tools on
//! the tab, less what this model and this call cannot use.

use roder_api::tools::ToolSpec;
use roder_ext_chrome::direct::direct_tool_specs;

use super::run::PREFIX;

/// The tools for one fallback.
///
/// `pictures` says the model is shown the picture a tool returns
/// ([`super::model::FallbackModel::sees_tool_result_images`]). The screenshot
/// tool returns its picture as a tool result, so without that it is not
/// offered: it would spend a step on a picture the model never sees.
///
/// `authorizes` says the caller authorized an irreversible step. Only then is
/// the model offered the switch that marks a press as that step; otherwise it
/// is not there to set (the live corpus saw a model set it on its own).
pub(crate) fn offered_tools(pictures: bool, authorizes: bool) -> Vec<ToolSpec> {
    let screenshot = format!("{PREFIX}_screenshot");
    direct_tool_specs(PREFIX, "the tab Jev was working in")
        .into_iter()
        .filter(|spec| pictures || spec.name != screenshot)
        .map(|mut spec| {
            if !authorizes && let Some(properties) = spec.parameters["properties"].as_object_mut() {
                properties.remove("authorize_irreversible");
            }
            spec
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tools: &[ToolSpec]) -> Vec<&str> {
        tools.iter().map(|tool| tool.name.as_str()).collect()
    }

    #[test]
    fn the_screenshot_tool_is_offered_only_with_pictures_and_nothing_else_goes() {
        let with = offered_tools(true, false);
        let without = offered_tools(false, false);
        assert!(names(&with).contains(&"jev_tab_screenshot"));
        assert!(!names(&without).contains(&"jev_tab_screenshot"));
        let others = names(&with)
            .into_iter()
            .filter(|name| *name != "jev_tab_screenshot")
            .collect::<Vec<_>>();
        assert_eq!(others, names(&without));
        assert!(others.contains(&"jev_tab_look") && others.contains(&"jev_tab_click"));
    }

    #[test]
    fn the_authorization_switch_is_offered_only_on_an_authorized_call() {
        let has_switch = |tools: &[ToolSpec]| {
            tools
                .iter()
                .find(|tool| tool.name == "jev_tab_click")
                .is_some_and(|tool| {
                    tool.parameters["properties"]["authorize_irreversible"].is_object()
                })
        };
        assert!(has_switch(&offered_tools(true, true)));
        assert!(!has_switch(&offered_tools(true, false)));
        assert!(!has_switch(&offered_tools(false, false)));
    }
}
