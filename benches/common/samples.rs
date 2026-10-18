//! Representative per-language samples for the parsing benchmark. Kept apart
//! from the fixture generator so each bench binary compiles only the helpers
//! it uses (no dead-code waiver needed).

// Representative per-language samples for the parsing benchmarks: ~50 lines
// each with the constructs that make parsing non-trivial (generics,
// decorators, arrow functions, interfaces, docstrings, match arms).

pub(crate) const RUST_SAMPLE: &str = r#"
use std::collections::HashMap;
use std::sync::Arc;

pub trait Provider {
    fn name(&self) -> &'static str;
    fn resolve(&self, key: &str) -> Option<String>;
}

pub struct Registry {
    providers: HashMap<String, Arc<dyn Provider>>,
    fallback: Option<String>,
}

impl Registry {
    pub fn new(fallback: Option<String>) -> Self {
        Self { providers: HashMap::new(), fallback }
    }

    pub fn register(&mut self, key: String, provider: Arc<dyn Provider>) {
        self.providers.insert(key, provider);
    }

    pub fn resolve(&self, key: &str) -> Option<String> {
        if let Some(provider) = self.providers.get(key) {
            return provider.resolve(key);
        }
        self.fallback.clone()
    }

    pub fn stats(&self) -> (usize, usize) {
        let total: usize = self.providers.iter().map(|(_, p)| p.name().len()).sum();
        (self.providers.len(), total)
    }
}

pub enum Status { Ready, Busy(usize), Failed { reason: String } }

pub fn describe(status: &Status) -> String {
    match status {
        Status::Ready => "ready".into(),
        Status::Busy(n) => format!("busy with {n} tasks"),
        Status::Failed { reason } => format!("failed: {reason}"),
    }
}

pub fn shared_helper(value: u64) -> u64 {
    let mut map = HashMap::new();
    map.insert("key", value);
    map.get("key").copied().unwrap_or(0)
}
"#;

pub(crate) const PYTHON_SAMPLE: &str = r#"
"""Module-level docstring for the sample."""
import os
from dataclasses import dataclass, field
from typing import Optional

@dataclass
class Widget:
    """A widget with a name and a value."""
    name: str
    value: int = 0
    tags: list[str] = field(default_factory=list)

    def scaled(self, factor: int) -> int:
        return self.value * factor

class Registry:
    def __init__(self) -> None:
        self._widgets: dict[str, Widget] = {}

    def add(self, widget: Widget) -> None:
        self._widgets[widget.name] = widget

    def get(self, name: str) -> Optional[Widget]:
        return self._widgets.get(name)

def shared_helper(value: int) -> int:
    return value * 2

def classify(value: int) -> str:
    if value < 0:
        return "negative"
    if value == 0:
        return "zero"
    return "positive"
"#;

pub(crate) const JS_SAMPLE: &str = r#"
import { readFile } from 'fs/promises';

export class Widget {
  constructor(name, value = 0) {
    this.name = name;
    this.value = value;
  }

  scaled(factor) {
    return this.value * factor;
  }
}

export function shared_helper(value) {
  return value * 2;
}

export async function loadWidget(path) {
  const raw = await readFile(path, 'utf8');
  const parsed = JSON.parse(raw);
  return new Widget(parsed.name, parsed.value);
}

export function classify(value) {
  if (value < 0) return 'negative';
  if (value === 0) return 'zero';
  return 'positive';
}
"#;

pub(crate) const TS_SAMPLE: &str = r#"
import { readFile } from 'fs/promises';

export interface Widget {
  name: string;
  value: number;
}

export class WidgetImpl implements Widget {
  constructor(public name: string, public value: number = 0) {}

  scaled(factor: number): number {
    return this.value * factor;
  }
}

export function shared_helper<T>(value: T): T {
  return value;
}

export async function loadWidget(path: string): Promise<Widget> {
  const raw = await readFile(path, 'utf8');
  const parsed = JSON.parse(raw) as Widget;
  return parsed;
}

export type Status = 'ready' | 'busy' | 'failed';
"#;
