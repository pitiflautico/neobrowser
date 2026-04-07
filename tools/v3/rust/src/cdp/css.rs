//! CDP CSS domain — computed styles, pseudo states, style manipulation.

use super::{CdpResult, CdpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CSSStyle {
    pub css_properties: Vec<CSSProperty>,
    pub shorthand_entries: Vec<ShorthandEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_sheet_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub css_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SourceRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CSSProperty {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub important: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<SourceRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShorthandEntry {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub important: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    pub start_line: i32,
    pub start_column: i32,
    pub end_line: i32,
    pub end_column: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CSSComputedStyleProperty {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleMatch {
    pub rule: Value,
    pub matching_selectors: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PseudoElementMatches {
    pub pseudo_type: String,
    pub matches: Vec<RuleMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformFontUsage {
    pub family_name: String,
    pub glyph_count: f64,
    pub is_custom_font: bool,
}

// ── Methods ────────────────────────────────────────────────────────

pub async fn enable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("CSS.enable", json!({})).await?;
    Ok(())
}

pub async fn disable(transport: &dyn CdpTransport) -> CdpResult<()> {
    transport.send("CSS.disable", json!({})).await?;
    Ok(())
}

pub async fn get_computed_style_for_node(
    transport: &dyn CdpTransport,
    node_id: i64,
) -> CdpResult<Vec<CSSComputedStyleProperty>> {
    let raw = transport
        .send("CSS.getComputedStyleForNode", json!({ "nodeId": node_id }))
        .await?;
    let props: Vec<CSSComputedStyleProperty> =
        serde_json::from_value(raw["computedStyle"].clone())?;
    Ok(props)
}

pub async fn get_matched_styles_for_node(
    transport: &dyn CdpTransport,
    node_id: i64,
) -> CdpResult<Value> {
    let raw = transport
        .send("CSS.getMatchedStylesForNode", json!({ "nodeId": node_id }))
        .await?;
    Ok(raw)
}

pub async fn get_inline_styles_for_node(
    transport: &dyn CdpTransport,
    node_id: i64,
) -> CdpResult<(Option<CSSStyle>, Option<CSSStyle>)> {
    let raw = transport
        .send("CSS.getInlineStylesForNode", json!({ "nodeId": node_id }))
        .await?;
    let inline_style: Option<CSSStyle> = raw
        .get("inlineStyle")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let attributes_style: Option<CSSStyle> = raw
        .get("attributesStyle")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    Ok((inline_style, attributes_style))
}

pub async fn force_pseudo_state(
    transport: &dyn CdpTransport,
    node_id: i64,
    forced_pseudo_classes: &[&str],
) -> CdpResult<()> {
    transport
        .send(
            "CSS.forcePseudoState",
            json!({
                "nodeId": node_id,
                "forcedPseudoClasses": forced_pseudo_classes,
            }),
        )
        .await?;
    Ok(())
}

pub async fn set_style_texts(
    transport: &dyn CdpTransport,
    edits: Vec<Value>,
) -> CdpResult<Vec<CSSStyle>> {
    let raw = transport
        .send("CSS.setStyleTexts", json!({ "edits": edits }))
        .await?;
    let styles: Vec<CSSStyle> = serde_json::from_value(raw["styles"].clone())?;
    Ok(styles)
}

pub async fn add_rule(
    transport: &dyn CdpTransport,
    style_sheet_id: &str,
    rule_text: &str,
    location: Value,
) -> CdpResult<Value> {
    let raw = transport
        .send(
            "CSS.addRule",
            json!({
                "styleSheetId": style_sheet_id,
                "ruleText": rule_text,
                "location": location,
            }),
        )
        .await?;
    Ok(raw)
}

pub async fn get_platform_fonts_for_node(
    transport: &dyn CdpTransport,
    node_id: i64,
) -> CdpResult<Vec<PlatformFontUsage>> {
    let raw = transport
        .send(
            "CSS.getPlatformFontsForNode",
            json!({ "nodeId": node_id }),
        )
        .await?;
    let fonts: Vec<PlatformFontUsage> = serde_json::from_value(raw["fonts"].clone())?;
    Ok(fonts)
}

pub async fn get_background_colors(
    transport: &dyn CdpTransport,
    node_id: i64,
) -> CdpResult<Value> {
    let raw = transport
        .send("CSS.getBackgroundColors", json!({ "nodeId": node_id }))
        .await?;
    Ok(raw)
}

pub async fn create_style_sheet(
    transport: &dyn CdpTransport,
    frame_id: &str,
) -> CdpResult<String> {
    let raw = transport
        .send("CSS.createStyleSheet", json!({ "frameId": frame_id }))
        .await?;
    let id = raw["styleSheetId"]
        .as_str()
        .ok_or("missing styleSheetId")?
        .to_string();
    Ok(id)
}

pub async fn set_effective_property_value_for_node(
    transport: &dyn CdpTransport,
    node_id: i64,
    property_name: &str,
    value: &str,
) -> CdpResult<()> {
    transport
        .send(
            "CSS.setEffectivePropertyValueForNode",
            json!({
                "nodeId": node_id,
                "propertyName": property_name,
                "value": value,
            }),
        )
        .await?;
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn test_enable() {
        let mock = MockTransport::new();
        mock.expect("CSS.enable", json!({})).await;

        enable(&mock).await.unwrap();
        mock.assert_called_once("CSS.enable").await;
    }

    #[tokio::test]
    async fn test_get_computed_style() {
        let mock = MockTransport::new();
        mock.expect(
            "CSS.getComputedStyleForNode",
            json!({
                "computedStyle": [
                    { "name": "color", "value": "rgb(0, 0, 0)" },
                    { "name": "font-size", "value": "16px" },
                ]
            }),
        )
        .await;

        let props = get_computed_style_for_node(&mock, 42).await.unwrap();
        assert_eq!(props.len(), 2);
        assert_eq!(props[0].name, "color");
        assert_eq!(props[0].value, "rgb(0, 0, 0)");
        assert_eq!(props[1].name, "font-size");
        assert_eq!(props[1].value, "16px");

        let params = mock
            .call_params("CSS.getComputedStyleForNode", 0)
            .await
            .unwrap();
        assert_eq!(params["nodeId"], 42);
    }

    #[tokio::test]
    async fn test_force_pseudo_state_hover() {
        let mock = MockTransport::new();
        mock.expect("CSS.forcePseudoState", json!({})).await;

        force_pseudo_state(&mock, 10, &["hover"]).await.unwrap();

        let params = mock
            .call_params("CSS.forcePseudoState", 0)
            .await
            .unwrap();
        assert_eq!(params["nodeId"], 10);
        assert_eq!(params["forcedPseudoClasses"], json!(["hover"]));
    }

    #[tokio::test]
    async fn test_force_pseudo_state_multiple() {
        let mock = MockTransport::new();
        mock.expect("CSS.forcePseudoState", json!({})).await;

        force_pseudo_state(&mock, 20, &["hover", "focus"]).await.unwrap();

        let params = mock
            .call_params("CSS.forcePseudoState", 0)
            .await
            .unwrap();
        assert_eq!(params["nodeId"], 20);
        assert_eq!(params["forcedPseudoClasses"], json!(["hover", "focus"]));
    }

    #[tokio::test]
    async fn test_get_inline_styles() {
        let mock = MockTransport::new();
        mock.expect(
            "CSS.getInlineStylesForNode",
            json!({
                "inlineStyle": {
                    "cssProperties": [
                        { "name": "color", "value": "red" }
                    ],
                    "shorthandEntries": []
                }
            }),
        )
        .await;

        let (inline, attrs) = get_inline_styles_for_node(&mock, 5).await.unwrap();
        assert!(inline.is_some());
        let style = inline.unwrap();
        assert_eq!(style.css_properties[0].name, "color");
        assert_eq!(style.css_properties[0].value, "red");
        assert!(attrs.is_none());
    }

    #[tokio::test]
    async fn test_get_platform_fonts() {
        let mock = MockTransport::new();
        mock.expect(
            "CSS.getPlatformFontsForNode",
            json!({
                "fonts": [
                    {
                        "familyName": "Arial",
                        "glyphCount": 42.0,
                        "isCustomFont": false
                    },
                    {
                        "familyName": "MyFont",
                        "glyphCount": 10.0,
                        "isCustomFont": true
                    }
                ]
            }),
        )
        .await;

        let fonts = get_platform_fonts_for_node(&mock, 7).await.unwrap();
        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].family_name, "Arial");
        assert!(!fonts[0].is_custom_font);
        assert_eq!(fonts[1].family_name, "MyFont");
        assert!(fonts[1].is_custom_font);
    }

    #[tokio::test]
    async fn test_create_style_sheet() {
        let mock = MockTransport::new();
        mock.expect(
            "CSS.createStyleSheet",
            json!({ "styleSheetId": "sheet-abc-123" }),
        )
        .await;

        let id = create_style_sheet(&mock, "frame-1").await.unwrap();
        assert_eq!(id, "sheet-abc-123");

        let params = mock
            .call_params("CSS.createStyleSheet", 0)
            .await
            .unwrap();
        assert_eq!(params["frameId"], "frame-1");
    }

    #[tokio::test]
    async fn test_set_effective_property_value() {
        let mock = MockTransport::new();
        mock.expect("CSS.setEffectivePropertyValueForNode", json!({}))
            .await;

        set_effective_property_value_for_node(&mock, 99, "display", "none")
            .await
            .unwrap();

        let params = mock
            .call_params("CSS.setEffectivePropertyValueForNode", 0)
            .await
            .unwrap();
        assert_eq!(params["nodeId"], 99);
        assert_eq!(params["propertyName"], "display");
        assert_eq!(params["value"], "none");
    }
}
