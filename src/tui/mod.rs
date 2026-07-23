mod component;
mod debug_pane;
mod message;
pub mod sessions_list;

use crate::{config::Config, globals};
use component::{Component, ComponentKind};
use debug_pane::DebugPane;
use message::Message;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
};
use sessions_list::SessionsList;
use std::env;
use std::io;

#[derive(Default)]
pub struct Tui {
    exit: bool,
    components: Vec<Component>,
    focused: ComponentKind,
}

impl Tui {
    pub fn new(c: Config) -> Self {
        let paths: Vec<&str> = c.paths.iter().map(String::as_str).collect();
        let selected: Option<usize> = Some(0);

        Tui {
            exit: false,
            components: vec![
                Component::Sessions(SessionsList::new(&paths, selected)),
                Component::Debug(DebugPane::new(globals::LOG_FILE)),
            ],
            focused: ComponentKind::default(),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn find_mut(&mut self, kind: ComponentKind) -> Option<&mut Component> {
        self.components.iter_mut().find(|c| c.kind() == kind)
    }

    pub fn focused_mut(&mut self) -> Option<&mut Component> {
        let focused = self.focused;
        self.find_mut(focused)
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if let Some(c) = self.focused_mut() {
            match c.handle_events()? {
                Message::Exit => self.exit(),
                _ => {}
            };
            Ok(())
        } else {
            panic!("there should be a component here");
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        // ┏━━━━━━━┓
        // ┃ list  ┃
        // ┃       ┃
        // ┃       ┃
        // ┃       ┃
        // ┗━━━━━━━┛
        // ┏━━━━━━━┓
        // ┃ debug ┃
        // ┗━━━━━━━┛

        let mut constraints = vec![Constraint::Fill(1)];
        if env::var("TUI_DEBUG").is_ok() {
            constraints = vec![Constraint::Fill(1), Constraint::Max(10)];
        }

        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(frame.area());

        frame.render_widget(self.find_mut(ComponentKind::Sessions), areas[0]);

        if env::var("TUI_DEBUG").is_ok() {
            frame.render_widget(self.find_mut(ComponentKind::Debug), areas[1]);
        }
    }
}
