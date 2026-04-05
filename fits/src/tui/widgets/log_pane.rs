use std::iter;

use ratatui::layout::Size;
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_scrollview::{ScrollView, ScrollViewState};

pub struct LogPane {}

impl StatefulWidget for LogPane {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // 100 lines of text
        let line_numbers = (1..=100).map(|i| format!("{:>3} ", i)).collect::<String>();
        let content = iter::repeat("Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n")
            .take(100)
            .collect::<String>();

        let content_size = Size::new(100, 30);
        let mut scroll_view = ScrollView::new(content_size);

        // the layout doesn't have to be hardcoded like this, this is just an example
        scroll_view.render_widget(Paragraph::new(line_numbers), Rect::new(0, 0, 5, 100));
        scroll_view.render_widget(Paragraph::new(content), Rect::new(5, 0, 95, 100));

        scroll_view.render(buf.area, buf, state);
    }
}
