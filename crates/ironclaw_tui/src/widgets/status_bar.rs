//! Status bar widget: model, tokens, cost, keybind hints.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::layout::TuiSlot;
use crate::render::format_tokens;
use crate::theme::Theme;

use super::{ActiveTab, AppState, TuiWidget};

pub struct StatusBarWidget {
    theme: Theme,
}

impl StatusBarWidget {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }
}

impl TuiWidget for StatusBarWidget {
    fn id(&self) -> &str {
        "status_bar"
    }

    fn slot(&self) -> TuiSlot {
        TuiSlot::StatusBarLeft
    }

    fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let total_tokens = state.total_input_tokens + state.total_output_tokens;
        let tokens_str = format_tokens(total_tokens);

        let sep = Span::styled(" \u{2502} ", self.theme.dim_style());

        let tab_label = match state.active_tab {
            ActiveTab::Conversation => "[Chat]",
            ActiveTab::Logs => "[Logs]",
        };

        let mut left_spans = vec![
            Span::styled(
                format!(" {tab_label} "),
                self.theme.bold_accent_style(),
            ),
            sep.clone(),
            Span::styled(
                state.model.to_string(),
                self.theme.accent_style(),
            ),
            sep.clone(),
            Span::styled(
                format!("{tokens_str} tokens"),
                self.theme.dim_style(),
            ),
        ];

        if state.total_cost_usd != "$0.00" {
            left_spans.push(sep.clone());
            left_spans.push(Span::styled(
                state.total_cost_usd.clone(),
                self.theme.dim_style(),
            ));
        }

        let right_text = "^L logs  ^B sidebar  ^C quit";
        let right_span = Span::styled(
            format!("{right_text}  "),
            self.theme.dim_style(),
        );

        // Render left-aligned portion
        let left_line = Line::from(left_spans);
        let left_widget = ratatui::widgets::Paragraph::new(left_line)
            .style(self.theme.status_style());
        left_widget.render(area, buf);

        // Render right-aligned keybind hints
        let right_line = Line::from(right_span);
        let right_widget = ratatui::widgets::Paragraph::new(right_line)
            .alignment(Alignment::Right)
            .style(self.theme.status_style());
        right_widget.render(area, buf);
    }
}
