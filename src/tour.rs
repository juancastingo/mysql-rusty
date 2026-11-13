#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourAnchor {
    SchemaTree = 0,
    SqlEditor = 1,
    RunButton = 2,
    ResultsGrid = 3,
    AiPanel = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TourStep {
    pub index: usize,
    pub title: &'static str,
    pub body: &'static str,
    pub anchor: TourAnchor,
}

pub const TOUR_STEPS: [TourStep; 5] = [
    TourStep {
        index: 0,
        title: "1. Explore the Schema Tree",
        body: "Browse tables, columns, views, and routines in the demo database. Click any table to inspect or query it.",
        anchor: TourAnchor::SchemaTree,
    },
    TourStep {
        index: 1,
        title: "2. Write SQL with Smart Autocomplete",
        body: "We pre-filled a sample query on `users`. Press Tab for inline completions, Ctrl+I for NL2SQL, or Ctrl+Shift+F to format.",
        anchor: TourAnchor::SqlEditor,
    },
    TourStep {
        index: 2,
        title: "3. Execute Your Query",
        body: "Click Run in the toolbar or press Ctrl+Enter (F5) to execute your query and stream results.",
        anchor: TourAnchor::RunButton,
    },
    TourStep {
        index: 3,
        title: "4. Inspect & Export Results",
        body: "Sort, filter, inspect cells, edit values inline, or export results to CSV, JSON, Markdown, or SQL INSERTs.",
        anchor: TourAnchor::ResultsGrid,
    },
    TourStep {
        index: 4,
        title: "5. Ask the AI Copilot",
        body: "Open the AI panel to generate SQL from natural language, explain queries, or diagnose slow EXPLAIN plans.",
        anchor: TourAnchor::AiPanel,
    },
];

#[derive(Debug, Clone, Default)]
pub struct TourState {
    pub show_welcome: bool,
    pub active_step: Option<usize>,
    pub anchor_rects: [Option<egui::Rect>; 5],
}

impl TourState {
    pub fn start_tour(&mut self) {
        self.show_welcome = false;
        self.active_step = Some(0);
    }

    pub fn current_step(&self) -> Option<&'static TourStep> {
        self.active_step.and_then(|idx| TOUR_STEPS.get(idx))
    }

    /// Advances to the next step. Returns `true` when the tour completes.
    pub fn next_step(&mut self) -> bool {
        match self.active_step {
            Some(idx) if idx + 1 < TOUR_STEPS.len() => {
                self.active_step = Some(idx + 1);
                false
            }
            Some(_) => {
                self.active_step = None;
                true
            }
            None => false,
        }
    }

    pub fn prev_step(&mut self) {
        if let Some(idx) = self.active_step
            && idx > 0
        {
            self.active_step = Some(idx - 1);
        }
    }

    pub fn skip(&mut self) {
        self.show_welcome = false;
        self.active_step = None;
    }

    pub fn set_anchor(&mut self, anchor: TourAnchor, rect: egui::Rect) {
        self.anchor_rects[anchor as usize] = Some(rect);
    }

    pub fn current_anchor_rect(&self) -> Option<egui::Rect> {
        self.current_step()
            .and_then(|s| self.anchor_rects[s.anchor as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tour_step_navigation_and_completion() {
        let mut tour = TourState {
            show_welcome: true,
            ..Default::default()
        };
        tour.start_tour();
        assert!(!tour.show_welcome);
        assert_eq!(tour.current_step().map(|s| s.index), Some(0));

        assert!(!tour.next_step());
        assert_eq!(tour.current_step().map(|s| s.index), Some(1));

        tour.prev_step();
        assert_eq!(tour.current_step().map(|s| s.index), Some(0));
        tour.prev_step();
        assert_eq!(tour.current_step().map(|s| s.index), Some(0));

        assert!(!tour.next_step()); // 1
        assert!(!tour.next_step()); // 2
        assert!(!tour.next_step()); // 3
        assert!(!tour.next_step()); // 4
        assert!(tour.next_step()); // completed
        assert!(tour.current_step().is_none());
    }

    #[test]
    fn test_tour_skip_clears_state() {
        let mut tour = TourState {
            show_welcome: true,
            active_step: Some(2),
            ..Default::default()
        };
        tour.skip();
        assert!(!tour.show_welcome);
        assert!(tour.active_step.is_none());
    }
}
