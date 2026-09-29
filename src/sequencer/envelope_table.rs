//! `SiMMLEnvelopeTable` (`libSiON-cpp/src/sequencer/simml_envelope_table.{h,cpp}`).
//!
//! Owns a `SinglyLinkedList<int>` (port: [`SinglyLinkedList`] from
//! `utils/translator_util`) whose last element is normally looped back
//! (`set_data` / `from_vector`), giving the envelope-walk "clamp at tail"
//! behavior. The list carries its own cursor (C++ `Element *_cursor`);
//! consumer-side `Element *` cursors live in `effector::EnvelopeCursor` and
//! walk read-only via `next_of`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::utils::translator_util::{SinglyLinkedList, TranslatorUtil};

#[derive(Default)]
pub struct SiMMLEnvelopeTable {
    pub data: Option<Rc<RefCell<SinglyLinkedList>>>,

}

impl SiMMLEnvelopeTable {
    /// C++ ctor `SiMMLEnvelopeTable(std::vector<int> p_table, int p_loop_point)`.
    pub fn new(p_table: Vec<i32>, p_loop_point: i32) -> Self {
        let mut table = SiMMLEnvelopeTable::default();
        table.from_vector(p_table, p_loop_point);
        table
    }

    /// C++ `set_data`. Takes ownership of the list (C++ `delete`s the old one),
    /// resets the cursor and forces the last element to loop.
    pub fn set_data(&mut self, mut p_data: SinglyLinkedList) {
        if p_data.is_empty() {
            self.data = Some(Rc::new(RefCell::new(p_data)));
            return;
        }
        p_data.front();
        let tail = p_data.len() - 1;
        p_data.loop_at(Some(tail));
        self.data = Some(Rc::new(RefCell::new(p_data)));
    }

    /// C++ `get_head()`: cursor-less head element, `None` = nullptr.
    pub fn get_head(&self) -> Option<usize> {
        match &self.data {
            None => None,
            Some(list) => {
                let list = list.borrow();
                if list.is_empty() {
                    None
                } else {
                    Some(0)
                }
            }
        }
    }

    /// C++ `get_tail()`.
    pub fn get_tail(&self) -> Option<usize> {
        match &self.data {
            None => None,
            Some(list) => {
                let list = list.borrow();
                if list.is_empty() {
                    None
                } else {
                    Some(list.len() - 1)
                }
            }
        }
    }

    /// C++ `parse_mml` (delegates to `TranslatorUtil::parse_table_numbers`).
    pub fn parse_mml(&mut self, p_table_numbers: &str, p_postfix: &str, p_max_index: i32) {
        let result = TranslatorUtil::parse_table_numbers(p_table_numbers, p_postfix, p_max_index);
        self.set_data(result.data);
    }

    /// C++ `from_vector`.
    pub fn from_vector(&mut self, p_table: Vec<i32>, p_loop_point: i32) {
        if p_table.is_empty() {
            self.data = None;
            return;
        }
        let mut list = SinglyLinkedList::new_empty();
        for value in p_table.into_iter() {
            list.append(value);
        }
        list.front();
        if p_loop_point >= 0 && (p_loop_point as usize) < list.len() {
            list.loop_at(Some(p_loop_point as usize));
        }
        self.data = Some(Rc::new(RefCell::new(list)));
    }

    /// C++ `to_vector`: walks the list's own cursor, clamping each value;
    /// past the end of a non-looped list the value stays `0`.
    pub fn to_vector(&mut self, p_length: usize, r_destination: &mut Vec<i32>, p_min: i32, p_max: i32) {
        r_destination.clear();
        r_destination.resize(p_length, 0);
        let Some(list) = self.data.clone() else {
            for slot in r_destination.iter_mut() {
                *slot = 0_i32.clamp(p_min, p_max);
            }
            return;
        };
        let mut list = list.borrow_mut();
        list.front();
        for i in 0..p_length {
            let mut value = 0;
            if list.get().is_some() {
                value = list.get().unwrap();
                list.next();
            }
            r_destination[i] = value.clamp(p_min, p_max);
        }
    }

    /// C++ `copy_from` (does NOT copy the loop, per the C++ FIXME).
    pub fn copy_from(&mut self, p_source: &Rc<RefCell<SiMMLEnvelopeTable>>) {
        let source = p_source.borrow();
        let Some(source_list) = source.data.clone() else {
            self.data = None;
            return;
        };
        let mut list = SinglyLinkedList::new_empty();
        {
            let source_list = source_list.borrow();
            for i in 0..source_list.len() {
                list.append(source_list.value_at(i));
            }
        }
        drop(source);
        self.data = Some(Rc::new(RefCell::new(list)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_vector_clamps_and_loops_at_tail() {
        let mut table = SiMMLEnvelopeTable::new(vec![1, 2, 300, -300], 3);
        let mut out = Vec::new();
        table.to_vector(6, &mut out, -10, 10);
        // looped at index 3: walk yields 1,2,10,clamp(-300)=-10 then repeats
        // the looped tail element forever.
        assert_eq!(out, vec![1, 2, 10, -10, -10, -10]);
    }

    #[test]
    fn to_vector_zero_fills_past_nonlooped_tail() {
        let mut table = SiMMLEnvelopeTable::default();
        table.from_vector(vec![5, 6], -1);
        // from_vector with loop_point -1 leaves a non-looped list: walk past
        // the end returns null and the value stays 0.
        let mut out = Vec::new();
        table.to_vector(4, &mut out, -100, 100);
        assert_eq!(out, vec![5, 6, 0, 0]);
    }

    #[test]
    fn copy_from_drops_loop() {
        let src = Rc::new(RefCell::new(SiMMLEnvelopeTable::new(vec![7, 8], 1)));
        let mut dst = SiMMLEnvelopeTable::default();
        dst.copy_from(&src);
        let mut out = Vec::new();
        dst.to_vector(4, &mut out, -100, 100);
        assert_eq!(out, vec![7, 8, 0, 0]);
    }
}


