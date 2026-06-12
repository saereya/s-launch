use iced::{
    widget::{button, container, text_input},
    Background, Border, Color, Shadow,
};

use crate::config::{parse_color_iced, StyleConfig};

/// Pre-resolved colors derived from `StyleConfig` to avoid re-parsing hex every frame.
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub border_color: Color,
    pub border_radius: f32,
    pub border_width: f32,
    pub input_background: Color,
    pub input_foreground: Color,
    pub item_background: Color,
    pub item_foreground: Color,
    pub selected_background: Color,
    pub selected_foreground: Color,
    pub font_size: f32,
    pub item_height: u16,
    pub padding: u16,
}

impl Theme {
    pub fn from_style(s: &StyleConfig) -> Self {
        let bg = parse_color_iced(&s.background);
        let fg = parse_color_iced(&s.foreground);
        Self {
            background: bg,
            foreground: fg,
            border_color: parse_color_iced(&s.border_color),
            border_radius: s.border_radius,
            border_width: s.border_width,
            input_background: s.input.background.as_deref().map(parse_color_iced).unwrap_or(bg),
            input_foreground: s.input.foreground.as_deref().map(parse_color_iced).unwrap_or(fg),
            item_background: s.item.background.as_deref().map(parse_color_iced).unwrap_or(bg),
            item_foreground: s.item.foreground.as_deref().map(parse_color_iced).unwrap_or(fg),
            selected_background: s
                .item
                .selected
                .background
                .as_deref()
                .map(parse_color_iced)
                .unwrap_or_else(|| parse_color_iced("#313244")),
            selected_foreground: s.item.selected.foreground.as_deref().map(parse_color_iced).unwrap_or(fg),
            font_size: s.font_size,
            item_height: s.item_height,
            padding: s.padding,
        }
    }

    pub fn window_container_style(&self) -> container::Style {
        container::Style {
            background: Some(Background::Color(self.background)),
            border: Border {
                color: self.border_color,
                width: self.border_width,
                radius: self.border_radius.into(),
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 18.0,
            },
            text_color: Some(self.foreground),
        }
    }

    pub fn input_style(&self) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + '_ {
        let bg = self.input_background;
        let fg = self.input_foreground;
        let sel = Color { a: 0.3, ..self.selected_background };
        move |_theme, _status| text_input::Style {
            background: Background::Color(bg),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 6.0.into(),
            },
            icon: fg,
            placeholder: Color { a: 0.4, ..fg },
            value: fg,
            selection: sel,
        }
    }

    pub fn item_style(
        &self,
        selected: bool,
    ) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
        let normal_bg = self.item_background;
        let normal_fg = self.item_foreground;
        let sel_bg = self.selected_background;
        let sel_fg = self.selected_foreground;

        move |_theme, status| {
            let (bg, fg) = match (selected, status) {
                (true, _) | (false, button::Status::Hovered | button::Status::Pressed) => {
                    (sel_bg, sel_fg)
                }
                _ => (normal_bg, normal_fg),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: fg,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 6.0.into(),
                },
                shadow: Shadow::default(),
            }
        }
    }
}
