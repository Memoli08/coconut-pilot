//! Pure input matching; independent of evdev and testable from recorded events.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use crate::config::Binding;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeyEvent {
    pub code: u16,
    pub value: i32,
}
#[derive(Default, Debug)]
pub struct Output {
    pub forward: Vec<KeyEvent>,
    pub triggered: bool,
    pub emergency: bool,
}
pub fn modifier(k: u16) -> bool {
    matches!(k, 29 | 42 | 54 | 56 | 97 | 100 | 125 | 126)
}
pub struct Matcher {
    binding: Binding,
    physical: BTreeSet<u16>,
    pending: Vec<KeyEvent>,
    suppressed: BTreeSet<u16>,
    started: Option<u64>,
    fired: bool,
}
impl Matcher {
    pub fn new(binding: Binding) -> Self {
        Self {
            binding,
            physical: Default::default(),
            pending: vec![],
            suppressed: Default::default(),
            started: None,
            fired: false,
        }
    }
    pub fn tick(&mut self, now: u64) -> Output {
        let mut o = Output::default();
        if self.started.is_some_and(|t| now.saturating_sub(t) >= 50) {
            o.forward.append(&mut self.pending);
            self.started = None;
        }
        o
    }
    pub fn event(&mut self, e: KeyEvent, now: u64) -> Output {
        let mut o = self.tick(now);
        if e.value == 1 {
            self.physical.insert(e.code);
        } else if e.value == 0 {
            self.physical.remove(&e.code);
        }
        if [14, 1, 28].iter().all(|k| self.physical.contains(k)) {
            o.emergency = true;
            return o;
        }
        if self.suppressed.contains(&e.code) {
            if e.value == 0 {
                self.suppressed.remove(&e.code);
                if e.code == self.binding.trigger {
                    self.fired = false
                }
            }
            return o;
        }
        let belongs = self.binding.keys.contains(&e.code);
        if e.value == 1 && belongs {
            if self.pending.is_empty() {
                self.started = Some(now)
            }
            self.pending.push(e);
            let complete = self
                .binding
                .keys
                .iter()
                .all(|k| self.pending.iter().any(|p| p.code == *k && p.value == 1));
            if complete && !self.fired {
                self.suppressed.extend(self.binding.keys.iter().copied());
                self.pending.clear();
                self.started = None;
                self.fired = true;
                o.triggered = true;
            }
            return o;
        }
        if !self.pending.is_empty() {
            o.forward.append(&mut self.pending);
            self.started = None;
        }
        o.forward.push(e);
        o
    }
}
#[derive(Default)]
pub struct Learner {
    held: BTreeSet<u16>,
    peak: BTreeSet<u16>,
    start: Option<u64>,
    last_down: u64,
    invalid: bool,
}
impl Learner {
    pub fn event(&mut self, e: KeyEvent, now: u64) -> Option<Result<(Vec<u16>, u16), String>> {
        if e.value == 1 {
            if self.held.is_empty() {
                self.start = Some(now);
                self.peak.clear();
                self.invalid = false;
            }
            self.last_down = now;
            self.held.insert(e.code);
            self.peak.extend(self.held.iter().copied());
        }
        if e.value == 0 {
            self.held.remove(&e.code);
            if self.held.is_empty() && self.start.is_some() {
                let start = self.start.take().unwrap();
                let keys: Vec<_> = self.peak.iter().copied().collect();
                let main: Vec<_> = keys.iter().copied().filter(|k| !modifier(*k)).collect();
                if self.invalid
                    || main.len() != 1
                    || keys.len() > 5
                    || self.last_down.saturating_sub(start) >= 50
                {
                    return Some(Err(
                        "Use a single key or a modifier chord completed within 50 ms".into(),
                    ));
                }
                if matches!(main[0], 1 | 28) {
                    return None;
                }
                return Some(Ok((keys, main[0])));
            }
        }
        None
    }
}
pub fn describe(keys: &[u16]) -> String {
    keys.iter()
        .map(|k| match *k {
            29 => "Left Ctrl".into(),
            42 => "Left Shift".into(),
            54 => "Right Shift".into(),
            56 => "Left Alt".into(),
            97 => "Right Ctrl".into(),
            100 => "Right Alt".into(),
            125 => "Left Super".into(),
            126 => "Right Super".into(),
            183..=194 => format!("F{}", k - 170),
            583 => "Assistant".into(),
            _ => format!("Key {k}"),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub trait InputBackend {
    fn learn_with_updates<F>(&self, on_first: F) -> anyhow::Result<Binding>
    where
        F: FnMut(&str, &Binding);
}
pub struct NativeInput;
impl InputBackend for NativeInput {
    fn learn_with_updates<F>(&self, on_first: F) -> anyhow::Result<Binding>
    where
        F: FnMut(&str, &Binding),
    {
        crate::service::learn_with_updates(on_first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn matcher() -> Matcher {
        Matcher::new(Binding {
            device: "test".into(),
            keys: vec![42, 125, 193],
            trigger: 193,
        })
    }
    fn e(code: u16, value: i32) -> KeyEvent {
        KeyEvent { code, value }
    }
    #[test]
    fn chord_once() {
        let mut m = matcher();
        assert!(m.event(e(125, 1), 0).forward.is_empty());
        m.event(e(42, 1), 1);
        assert!(m.event(e(193, 1), 2).triggered);
        assert!(!m.event(e(193, 2), 3).triggered);
        for k in [193, 42, 125] {
            assert!(m.event(e(k, 0), 4).forward.is_empty());
        }
        m.event(e(125, 1), 10);
        m.event(e(42, 1), 11);
        assert!(m.event(e(193, 1), 12).triggered);
    }
    #[test]
    fn normal_shortcut() {
        let mut m = matcher();
        m.event(e(125, 1), 0);
        assert_eq!(m.event(e(30, 1), 2).forward, vec![e(125, 1), e(30, 1)]);
    }
    #[test]
    fn timeout() {
        let mut m = matcher();
        m.event(e(42, 1), 0);
        assert_eq!(m.tick(50).forward, vec![e(42, 1)]);
        assert_eq!(m.event(e(42, 0), 51).forward, vec![e(42, 0)]);
    }
    #[test]
    fn existing_modifier_not_swallowed() {
        let mut m = matcher();
        m.event(e(42, 1), 0);
        m.tick(60);
        m.event(e(125, 1), 61);
        assert!(!m.event(e(193, 1), 62).triggered);
    }
    #[test]
    fn learn() {
        let mut l = Learner::default();
        for (i, k) in [125, 42, 193].iter().enumerate() {
            assert!(l.event(e(*k, 1), i as u64).is_none())
        }
        l.event(e(193, 0), 10);
        l.event(e(42, 0), 11);
        assert_eq!(
            l.event(e(125, 0), 12).unwrap().unwrap(),
            (vec![42, 125, 193], 193)
        );
    }
    #[test]
    fn emergency() {
        let mut m = matcher();
        m.event(e(14, 1), 0);
        m.event(e(1, 1), 1);
        assert!(m.event(e(28, 1), 2).emergency);
    }
}
