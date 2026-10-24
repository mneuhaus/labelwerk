//! Batch mode: labels collected one after another (Enter puts the editor's label in) and printed as one job,
//! so the printer cuts between them and feeds the blank start of the tape only once.

use labelwerk_core::Label;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Batch {
    pub labels: Vec<Label>,
    /// Set while an entry is back in the editor: where it goes in again.
    pub editing: Option<usize>,
}

impl Batch {
    /// Where the editor's label goes: back to where it was taken from, else to the end.
    pub fn slot(&self) -> usize {
        self.editing.map_or(self.labels.len(), |i| i.min(self.labels.len()))
    }

    /// Put the editor's label in. Empty labels are not added; returns where it went.
    pub fn add(&mut self, label: Label) -> Option<usize> {
        if label.is_empty() {
            return None;
        }
        let at = self.slot();
        self.labels.insert(at, label);
        self.editing = None;
        Some(at)
    }

    /// Take entry `i` back into the editor. What the editor held goes into the batch first.
    pub fn edit(&mut self, i: usize, editor: Label) -> Option<Label> {
        let i = match self.add(editor) {
            Some(at) if at <= i => i + 1,
            _ => i,
        };
        if i >= self.labels.len() {
            return None;
        }
        self.editing = Some(i);
        Some(self.labels.remove(i))
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.labels.len() {
            self.labels.remove(i);
            if let Some(e) = self.editing.filter(|&e| e > i) {
                self.editing = Some(e - 1);
            }
        }
    }

    /// Everything to print, in order: the entries and the label still in the editor, at its slot.
    pub fn to_print(&self, editor: &Label) -> Vec<Label> {
        let mut all = self.labels.clone();
        if !editor.is_empty() {
            all.insert(self.slot(), editor.clone());
        }
        all
    }

    /// The first `n` labels of `to_print` came out of the printer; keep the rest. True when the editor's
    /// label was among them.
    pub fn printed(&mut self, editor_included: bool, n: usize) -> bool {
        let slot = self.slot();
        let mut all: Vec<Option<Label>> = self.labels.drain(..).map(Some).collect();
        if editor_included {
            all.insert(slot, None);
        }
        let rest = all.split_off(n.min(all.len()));
        // at the end, the editor's label simply follows the rest
        self.editing = rest.iter().position(Option::is_none).filter(|&i| i + 1 < rest.len());
        self.labels = rest.into_iter().flatten().collect();
        editor_included && slot < n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(text: &str) -> Label {
        Label { text: text.into(), ..Label::default() }
    }

    fn texts(labels: &[Label]) -> Vec<&str> {
        labels.iter().map(|l| l.text.as_str()).collect()
    }

    fn batch(entries: &[&str]) -> Batch {
        Batch { labels: entries.iter().map(|t| label(t)).collect(), editing: None }
    }

    #[test]
    fn enter_appends_and_skips_empty_labels() {
        let mut b = Batch::default();
        assert_eq!(b.add(label("A")), Some(0));
        assert_eq!(b.add(label("  ")), None);
        assert_eq!(b.add(Label { qr: true, qr_content: "https://x".into(), ..Label::default() }), Some(1));
        assert_eq!(b.add(label("B")), Some(2));
        assert_eq!(b.labels.len(), 3);
    }

    #[test]
    fn an_edited_entry_goes_back_to_its_place() {
        let mut b = batch(&["A", "B", "C"]);
        assert_eq!(b.edit(1, label("")).unwrap().text, "B");
        assert_eq!(texts(&b.to_print(&label("B2"))), ["A", "B2", "C"]);
        assert_eq!(b.add(label("B2")), Some(1));
        assert_eq!(texts(&b.labels), ["A", "B2", "C"]);
        assert_eq!(b.add(label("D")), Some(3), "after that, Enter appends again");
    }

    #[test]
    fn editing_another_entry_keeps_what_the_editor_held() {
        let mut b = batch(&["A", "B", "C"]);
        b.edit(0, label("")); // A in the editor, slot 0
        assert_eq!(b.edit(1, label("A1")).unwrap().text, "C", "index 1 meant C before A1 went back in front");
        assert_eq!(texts(&b.labels), ["A1", "B"]);
        assert_eq!(b.editing, Some(2));
        // typed but not added yet: goes in at the end before the next entry is taken
        let mut b = batch(&["A", "B"]);
        assert_eq!(b.edit(0, label("new")).unwrap().text, "A");
        assert_eq!(texts(&b.labels), ["B", "new"]);
    }

    #[test]
    fn removing_an_entry_keeps_the_editing_slot() {
        let mut b = batch(&["A", "B", "C"]);
        b.edit(2, label(""));
        b.remove(0);
        assert_eq!(b.slot(), 1);
        assert_eq!(texts(&b.to_print(&label("C"))), ["B", "C"]);
    }

    #[test]
    fn a_partial_print_keeps_what_did_not_come_out() {
        let mut b = batch(&["A", "B", "C"]);
        b.edit(1, label(""));
        let all = b.to_print(&label("B"));
        assert_eq!(texts(&all), ["A", "B", "C"]);
        assert!(!b.printed(true, 1), "A printed, then the tape ran out");
        assert_eq!(texts(&b.labels), ["C"]);
        assert_eq!(b.editing, Some(0), "B is still in the editor, in front of C");
        assert!(b.printed(true, 3));
        assert_eq!(b, Batch::default());
        let mut b = batch(&["A", "B"]);
        assert!(!b.printed(false, 2));
        assert!(b.labels.is_empty() && b.editing.is_none());
        let mut b = batch(&["A", "B"]);
        assert!(!b.printed(true, 0), "nothing came out");
        assert_eq!(b, batch(&["A", "B"]));
    }
}
