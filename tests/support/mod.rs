//! Shared deterministic replay through the same guard and terminal core used by Session.
#![allow(dead_code)] // Individual integration-test binaries use different parts of the fixture rig.
use alacritty_terminal::{event::Event, vte::ansi::Processor};
use crossbeam_channel::Receiver;
use serde_json::{Value, json};
use tide_terminal::{
    observer::{Marker, Observer, ProtocolGuard},
    terminal::{self, Core},
};

pub struct Replay {
    pub terminal: Core,
    pub observer: Observer,
    pub events: Receiver<Event>,
    guard: ProtocolGuard,
    core_parser: Processor,
    observer_parser: vte::Parser,
}
impl Replay {
    pub fn new(rows: usize, cols: usize) -> Self {
        let (terminal, events) = terminal::create(rows, cols);
        Self {
            terminal,
            events,
            observer: Observer::default(),
            guard: ProtocolGuard::default(),
            core_parser: Processor::new(),
            observer_parser: vte::Parser::new(),
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        let bytes = self.guard.filter(bytes);
        self.core_parser.advance(&mut self.terminal, &bytes);
        self.observer_parser.advance(&mut self.observer, &bytes);
    }
    pub fn fingerprint(&self) -> Value {
        let view = terminal::snapshot(&self.terminal);
        json!({"rows":view.rows,"cols":view.cols,"cursor":view.cursor,
            "shape":format!("{:?}", view.cursor_shape),"mode":view.mode.bits(),"offset":view.offset,
            "cells":view.cells.iter().map(|c| json!([c.row,c.col,c.text,c.fg,c.bg,c.wide,c.underline,c.strike])).collect::<Vec<_>>(),
            "observer":self.observer.text(),"tail":self.observer.tail(),"hint":self.observer.input_hint(),
            "alternate":self.observer.alternate,"markers":markers(&self.observer.markers),"bell":self.observer.bell})
    }
    pub fn lines(&self) -> Vec<String> {
        let view = terminal::snapshot(&self.terminal);
        let mut rows = vec![vec![" ".to_owned(); view.cols]; view.rows];
        for cell in view.cells {
            rows[cell.row][cell.col] = cell.text;
            if cell.wide && cell.col + 1 < view.cols {
                rows[cell.row][cell.col + 1].clear();
            }
        }
        rows.into_iter()
            .map(|row| row.concat().trim_end().to_owned())
            .collect()
    }
}
pub fn markers(markers: &[Marker]) -> Vec<Value> {
    markers
        .iter()
        .map(|marker| match marker {
            Marker::Append => json!(["append"]),
            Marker::Prompt => json!(["prompt"]),
            Marker::Start => json!(["start"]),
            Marker::End(code) => json!(["end", code]),
            Marker::Command(s) => json!(["command", s]),
            Marker::Cwd(s) => json!(["cwd", s]),
            Marker::Input(s) => json!(["input", s]),
        })
        .collect()
}
