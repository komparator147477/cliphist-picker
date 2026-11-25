//! Marked entries, kept in the order they were marked (= paste order).

#[derive(Default, Debug)]
pub struct Marks(Vec<usize>);

impl Marks {
    pub fn toggle(&mut self, idx: usize) {
        match self.0.iter().position(|&i| i == idx) {
            Some(p) => {
                self.0.remove(p);
            }
            None => self.0.push(idx),
        }
    }

    /// 1-based mark number, if marked.
    pub fn number(&self, idx: usize) -> Option<usize> {
        self.0.iter().position(|&i| i == idx).map(|p| p + 1)
    }

    pub fn order(&self) -> &[usize] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_mark_order() {
        let mut m = Marks::default();
        m.toggle(4);
        m.toggle(1);
        m.toggle(7);
        assert_eq!(m.order(), &[4, 1, 7]);
        assert_eq!(m.number(1), Some(2));
        assert_eq!(m.number(3), None);
    }

    #[test]
    fn unmark_renumbers() {
        let mut m = Marks::default();
        m.toggle(4);
        m.toggle(1);
        m.toggle(7);
        m.toggle(4);
        assert_eq!(m.order(), &[1, 7]);
        assert_eq!(m.number(7), Some(2));
        assert_eq!(m.len(), 2);
    }
}
