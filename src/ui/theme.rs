use crate::config::{parse_color, StyleConfig};

/// Generate a self-contained CSS string from the user's StyleConfig.
/// Applied at STYLE_PROVIDER_PRIORITY_USER (800) so it overrides any system theme.
pub fn build_css(s: &StyleConfig) -> String {
    let item_bg = s.item.background.as_deref().unwrap_or(&s.background);
    let item_fg = s.item.foreground.as_deref().unwrap_or(&s.foreground);
    let sel_bg = s
        .item
        .selected
        .background
        .as_deref()
        .unwrap_or("#313244");
    let sel_fg = s.item.selected.foreground.as_deref().unwrap_or(&s.foreground);
    let input_bg = s.input.background.as_deref().unwrap_or(&s.background);
    let input_fg = s.input.foreground.as_deref().unwrap_or(&s.foreground);

    let [r, g, b, _] = parse_color(input_fg);
    let placeholder = format!("rgba({r},{g},{b},0.4)");

    let [dr, dg, db, _] = parse_color(item_fg);
    let desc_color = format!("rgba({dr},{dg},{db},0.6)");

    let desc_size = (s.font_size * 0.85) as u32;

    format!(
        r#"
window.slaunch {{
    background-color: {bg};
    border-radius: {border_radius}px;
    border: {border_width:.1}px solid {border_color};
}}

window.slaunch entry {{
    background-color: {input_bg};
    color: {input_fg};
    border: none;
    border-radius: 6px;
    box-shadow: none;
    outline: none;
    padding: {padding}px;
    min-height: 0;
    font-family: {font_family};
    font-size: {font_size:.1}px;
}}

window.slaunch entry:focus {{
    border: none;
    box-shadow: none;
    outline: none;
}}

window.slaunch entry > text {{
    color: {input_fg};
}}

window.slaunch entry placeholder {{
    color: {placeholder};
}}

window.slaunch listbox {{
    background-color: transparent;
    border: none;
    outline: none;
}}

window.slaunch listbox > row {{
    background-color: {item_bg};
    color: {item_fg};
    padding: 4px {padding}px;
    border-radius: 6px;
    border: none;
    outline: none;
    min-height: {item_height}px;
    font-family: {font_family};
    font-size: {font_size:.1}px;
}}

window.slaunch listbox > row:selected,
window.slaunch listbox > row:hover {{
    background-color: {sel_bg};
    color: {sel_fg};
    outline: none;
}}

window.slaunch .row-desc {{
    font-size: {desc_size}px;
    color: {desc_color};
}}
"#,
        bg = s.background,
        border_radius = s.border_radius as u32,
        border_width = s.border_width,
        border_color = s.border_color,
        input_bg = input_bg,
        input_fg = input_fg,
        padding = s.padding,
        font_family = s.font_family,
        font_size = s.font_size,
        item_bg = item_bg,
        item_fg = item_fg,
        item_height = s.item_height,
        sel_bg = sel_bg,
        sel_fg = sel_fg,
        placeholder = placeholder,
        desc_color = desc_color,
        desc_size = desc_size,
    )
}
