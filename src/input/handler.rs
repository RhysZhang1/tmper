use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};

use crate::event::AppEvent;

/// Turns a stream of keys into `AppEvent`s, resolving the `gg` / `dd`
/// double-key sequences.
///
/// The invariant that matters: **every key is delivered, exactly once, in
/// order**. `g` and `d` are held back for one keystroke because they may start
/// a sequence, so the handler has two ways to release a held key — the next
/// key arrives, or [`KeyHandler::flush_expired`] is called once the window has
/// passed (the app calls it on tick).
///
/// Text-entry modes (search, command line, playlist rename) must bypass this
/// entirely: there is no sequence to resolve while the user is typing, and a
/// swallowed `d`, a deleted track, or a `q` that quits the app mid-query are
/// all the same bug. The app decides that and calls it out of band.
pub struct KeyHandler {
    /// A `g`/`d` waiting to see whether the next key completes a sequence.
    pending: Option<(KeyEvent, Instant)>,
    timeout_ms: u64,
    quit_key: KeyEvent,
}

impl KeyHandler {
    pub fn new(timeout_ms: u64, quit_key: KeyEvent) -> Self {
        Self {
            pending: None,
            timeout_ms,
            quit_key,
        }
    }

    /// Feed one key; returns the events it produced, in order (0, 1 or 2).
    pub fn process(&mut self, key: KeyEvent) -> Vec<AppEvent> {
        // Non-standard ASCII control characters carry no meaning here.
        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_control() {
                return Vec::new();
            }
        }

        if key.code == self.quit_key.code && key.modifiers == self.quit_key.modifiers {
            self.pending = None;
            return vec![AppEvent::Quit];
        }

        let mut events = Vec::new();

        // A held prefix either completes a sequence with this key, or stands on
        // its own — in which case it is emitted *before* this key, preserving
        // order.
        if let Some((prev, at)) = self.pending.take() {
            let within_window = at.elapsed().as_millis() as u64 <= self.timeout_ms;
            if within_window {
                if let Some(event) = Self::sequence(&prev, &key) {
                    return vec![event];
                }
            }
            events.push(AppEvent::Key(prev));
        }

        // This key is now handled on its own merits, and may itself start a
        // sequence.
        if Self::starts_sequence(&key) {
            self.pending = Some((key, Instant::now()));
        } else {
            events.push(AppEvent::Key(key));
        }

        events
    }

    /// Release a held `g`/`d` once its window has passed, so a lone prefix is
    /// not stuck until the user happens to press something else. Returns `None`
    /// while the window is still open.
    pub fn flush_expired(&mut self) -> Option<AppEvent> {
        let (_, at) = self.pending.as_ref()?;
        if at.elapsed().as_millis() as u64 <= self.timeout_ms {
            return None;
        }
        let (key, _) = self.pending.take()?;
        Some(AppEvent::Key(key))
    }

    /// Drop any held key without emitting it. Used when the app switches into a
    /// text-entry mode, where a stale prefix must not surface later.
    pub fn discard_pending(&mut self) {
        self.pending = None;
    }

    /// Only the bare `g`/`d` keys start a sequence. Matching on `code` alone
    /// would also claim Ctrl+D — a global half-page-scroll binding — so two
    /// quick Ctrl+D presses would resolve to `dd` and delete a track.
    fn starts_sequence(key: &KeyEvent) -> bool {
        key.modifiers.is_empty() && matches!(key.code, KeyCode::Char('g') | KeyCode::Char('d'))
    }

    fn sequence(prev: &KeyEvent, next: &KeyEvent) -> Option<AppEvent> {
        if !next.modifiers.is_empty() {
            return None;
        }
        match (prev.code, next.code) {
            (KeyCode::Char('g'), KeyCode::Char('g')) => Some(AppEvent::JumpTop),
            (KeyCode::Char('d'), KeyCode::Char('d')) => Some(AppEvent::RemoveSelected),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::NONE)
    }

    fn quit() -> KeyEvent {
        key('q')
    }

    fn handler() -> KeyHandler {
        KeyHandler::new(200, quit())
    }

    fn codes(events: &[AppEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                AppEvent::Key(k) => match k.code {
                    KeyCode::Char(c) => c.to_string(),
                    other => format!("{other:?}"),
                },
                AppEvent::Quit => "Quit".into(),
                AppEvent::JumpTop => "JumpTop".into(),
                AppEvent::RemoveSelected => "RemoveSelected".into(),
                AppEvent::Tick => "Tick".into(),
            })
            .collect()
    }

    // ── Sequences ──

    #[test]
    fn gg_emits_jump_top() {
        let mut h = handler();
        assert!(h.process(key('g')).is_empty(), "prefix is held");
        assert_eq!(codes(&h.process(key('g'))), ["JumpTop"]);
    }

    #[test]
    fn dd_emits_remove_selected() {
        let mut h = handler();
        assert!(h.process(key('d')).is_empty());
        assert_eq!(codes(&h.process(key('d'))), ["RemoveSelected"]);
    }

    // ── Nothing is swallowed ──

    /// A `g`/`d` that does not start a real sequence must still be delivered,
    /// and the key that followed it must not be dropped. It used to be: the
    /// handler emitted the held key and returned, discarding the current one.
    #[test]
    fn non_sequence_delivers_both_keys_in_order() {
        let mut h = handler();
        assert!(h.process(key('d')).is_empty());
        assert_eq!(codes(&h.process(key('j'))), ["d", "j"]);
    }

    #[test]
    fn plain_keys_pass_through() {
        let mut h = handler();
        assert_eq!(codes(&h.process(key('j'))), ["j"]);
        assert_eq!(codes(&h.process(key('k'))), ["k"]);
    }

    /// Two different prefixes in a row: the first stands alone, the second is
    /// held. Order is preserved.
    #[test]
    fn prefix_followed_by_another_prefix() {
        let mut h = handler();
        assert!(h.process(key('g')).is_empty());
        assert_eq!(codes(&h.process(key('d'))), ["g"]);
        // The `d` is still held and can complete a sequence.
        assert_eq!(codes(&h.process(key('d'))), ["RemoveSelected"]);
    }

    // ── The held key is released by time, not only by the next key ──

    #[test]
    fn lone_prefix_flushes_once_the_window_passes() {
        let mut h = KeyHandler::new(0, quit());
        assert!(h.process(key('g')).is_empty());
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(
            h.flush_expired().map(|e| codes(&[e])),
            Some(vec!["g".to_string()]),
            "a lone `g` must be delivered rather than hanging until the next key"
        );
        // Nothing left over.
        assert!(h.flush_expired().is_none());
    }

    #[test]
    fn flush_is_none_while_the_window_is_open() {
        let mut h = KeyHandler::new(10_000, quit());
        assert!(h.process(key('g')).is_empty());
        assert!(h.flush_expired().is_none(), "still waiting for `gg`");
    }

    #[test]
    fn flush_is_none_with_nothing_pending() {
        let mut h = handler();
        assert!(h.flush_expired().is_none());
    }

    /// A prefix older than the window must not pair with the next key.
    #[test]
    fn stale_prefix_does_not_pair() {
        let mut h = KeyHandler::new(0, quit());
        assert!(h.process(key('g')).is_empty());
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(
            codes(&h.process(key('g'))),
            ["g"],
            "no JumpTop from a stale prefix"
        );
        // The second `g` is held in its own right.
        assert_eq!(codes(&h.process(key('g'))), ["JumpTop"]);
    }

    // ── Quit and hygiene ──

    #[test]
    fn quit_key_emits_quit() {
        let mut h = handler();
        assert_eq!(codes(&h.process(key('q'))), ["Quit"]);
    }

    #[test]
    fn quit_key_clears_a_held_prefix() {
        let mut h = handler();
        assert!(h.process(key('g')).is_empty());
        assert_eq!(codes(&h.process(key('q'))), ["Quit"]);
        assert!(h.flush_expired().is_none(), "no orphan `g` after quitting");
    }

    /// Literal control bytes are dropped; they carry no meaning here.
    #[test]
    fn literal_control_characters_are_filtered() {
        let mut h = handler();
        let ctrl_a_byte =
            KeyEvent::new(KeyCode::Char('\x01'), crossterm::event::KeyModifiers::NONE);
        assert!(h.process(ctrl_a_byte).is_empty());
    }

    /// A Ctrl-modified letter is a normal key at this layer — the filter above
    /// only drops literal control bytes — and must be delivered like any other.
    #[test]
    fn ctrl_modified_letters_pass_through() {
        let mut h = handler();
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), crossterm::event::KeyModifiers::CONTROL);
        assert_eq!(codes(&h.process(ctrl_d)), ["d"]);
    }

    /// Ctrl+D is bound to half-page scroll. It must never be read as a `dd`
    /// prefix — two quick presses used to resolve to RemoveSelected and delete
    /// a track.
    #[test]
    fn repeated_ctrl_d_never_deletes() {
        let mut h = handler();
        let ctrl_d = || KeyEvent::new(KeyCode::Char('d'), crossterm::event::KeyModifiers::CONTROL);
        assert_eq!(codes(&h.process(ctrl_d())), ["d"]);
        assert_eq!(codes(&h.process(ctrl_d())), ["d"], "not RemoveSelected");
    }

    #[test]
    fn discard_pending_drops_the_held_key() {
        let mut h = handler();
        assert!(h.process(key('d')).is_empty());
        h.discard_pending();
        assert!(h.flush_expired().is_none());
        assert!(
            h.process(key('d')).is_empty(),
            "the first `d` starts a fresh sequence"
        );
        assert_eq!(codes(&h.process(key('d'))), ["RemoveSelected"]);
    }
}
