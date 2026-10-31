// Native VM/page semantics derived from xsystem4 a85467d (C) Nunuhara Cabbage.
// GPL-2.0-or-later; see SOURCE.json. No image, text layout or device resources.
use anyhow::{Context, Result, ensure};
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageKind {
    Global,
    Local(usize),
    Structure(usize),
    Array {
        kind: i32,
        structure: i32,
        rank: usize,
    },
}
#[derive(Clone, Debug)]
pub struct Page {
    pub kind: PageKind,
    pub values: Vec<i32>,
}
#[derive(Clone, Debug)]
pub enum Object {
    String(Arc<Vec<u8>>),
    Page(Option<Page>),
}
#[derive(Clone, Debug)]
pub struct Slot {
    pub references: u32,
    pub sequence: u32,
    pub object: Object,
}
#[derive(Clone, Debug)]
pub struct Heap {
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    next_sequence: u32,
    values: usize,
}
pub const MAX_SLOTS: usize = 1_000_000;
pub const MAX_VALUES: usize = 8_000_000;
impl Heap {
    pub fn new(globals: usize) -> Result<Self> {
        ensure!(globals <= MAX_VALUES, "System4 global value budget");
        Ok(Self {
            slots: vec![Some(Slot {
                references: 1,
                sequence: 1,
                object: Object::Page(Some(Page {
                    kind: PageKind::Global,
                    values: vec![0; globals],
                })),
            })],
            free: Vec::new(),
            next_sequence: 2,
            values: globals,
        })
    }
    pub fn slot(&self, id: i32) -> Result<&Slot> {
        self.slots
            .get(usize::try_from(id)?)
            .and_then(Option::as_ref)
            .context("System4 invalid heap slot")
    }
    pub fn slot_mut(&mut self, id: i32) -> Result<&mut Slot> {
        self.slots
            .get_mut(usize::try_from(id)?)
            .and_then(Option::as_mut)
            .context("System4 invalid heap slot")
    }
    pub fn alloc(&mut self, object: Object) -> Result<i32> {
        let added = match &object {
            Object::Page(Some(p)) => p.values.len(),
            Object::String(s) => s.len().div_ceil(4),
            _ => 0,
        };
        ensure!(
            self.values
                .checked_add(added)
                .is_some_and(|v| v <= MAX_VALUES),
            "System4 heap value budget"
        );
        ensure!(
            self.next_sequence != u32::MAX,
            "System4 heap sequence exhausted"
        );
        let slot = Slot {
            references: 1,
            sequence: self.next_sequence,
            object,
        };
        let index = if let Some(i) = self.free.pop() {
            self.slots[i] = Some(slot);
            i
        } else {
            ensure!(self.slots.len() < MAX_SLOTS, "System4 heap slot budget");
            self.slots.push(Some(slot));
            self.slots.len() - 1
        };
        self.next_sequence += 1;
        self.values += added;
        Ok(index as i32)
    }
    pub fn page_opt(&self, id: i32) -> Result<Option<&Page>> {
        match &self.slot(id)?.object {
            Object::Page(p) => Ok(p.as_ref()),
            _ => anyhow::bail!("System4 expected page slot"),
        }
    }
    pub fn page(&self, id: i32) -> Result<&Page> {
        self.page_opt(id)?.context("System4 null page")
    }
    pub fn page_mut(&mut self, id: i32) -> Result<&mut Page> {
        match &mut self.slot_mut(id)?.object {
            Object::Page(Some(p)) => Ok(p),
            _ => anyhow::bail!("System4 expected non-null page"),
        }
    }
    pub fn string(&self, id: i32) -> Result<Arc<Vec<u8>>> {
        match &self.slot(id)?.object {
            Object::String(s) => Ok(s.clone()),
            _ => anyhow::bail!("System4 expected string slot"),
        }
    }
    pub fn get(&self, id: i32, index: i32) -> Result<i32> {
        self.page(id)?
            .values
            .get(usize::try_from(index)?)
            .copied()
            .context("System4 variable index")
    }
    pub fn set(&mut self, id: i32, index: i32, value: i32) -> Result<()> {
        *self
            .page_mut(id)?
            .values
            .get_mut(usize::try_from(index)?)
            .context("System4 variable index")? = value;
        Ok(())
    }
    pub fn reference(&mut self, id: i32) -> Result<()> {
        if id == -1 {
            return Ok(());
        }
        let s = self.slot_mut(id)?;
        s.references = s
            .references
            .checked_add(1)
            .context("System4 reference overflow")?;
        Ok(())
    }
    pub fn set_page(&mut self, id: i32, page: Option<Page>) -> Result<()> {
        let old = self.page_opt(id)?.map_or(0, |p| p.values.len());
        let new = page.as_ref().map_or(0, |p| p.values.len());
        ensure!(
            self.values - old + new <= MAX_VALUES,
            "System4 heap value budget"
        );
        self.values = self.values - old + new;
        self.slot_mut(id)?.object = Object::Page(page);
        Ok(())
    }
    pub fn set_string(&mut self, id: i32, string: Arc<Vec<u8>>) -> Result<()> {
        let old = self.string(id)?.len().div_ceil(4);
        let new = string.len().div_ceil(4);
        ensure!(
            self.values - old + new <= MAX_VALUES,
            "System4 string budget"
        );
        self.values = self.values - old + new;
        self.slot_mut(id)?.object = Object::String(string);
        Ok(())
    }
    pub fn free(&mut self, id: i32) -> Result<()> {
        let i = usize::try_from(id)?;
        let s = self
            .slots
            .get_mut(i)
            .context("System4 free slot")?
            .take()
            .context("System4 double free")?;
        self.values -= match s.object {
            Object::Page(Some(p)) => p.values.len(),
            Object::String(s) => s.len().div_ceil(4),
            _ => 0,
        };
        self.free.push(i);
        Ok(())
    }
    pub fn live_slots(&self) -> usize {
        self.slots.len() - self.free.len()
    }
    pub fn value_count(&self) -> usize {
        self.values
    }
}
pub fn is_array(k: i32) -> bool {
    matches!(k, 14..=17 | 30 | 50 | 58 | 66)
}
pub fn is_reference(k: i32) -> bool {
    matches!(k, 18..=25 | 31 | 32 | 51 | 52 | 59 | 60 | 67 | 69 | 80)
}
pub fn owns_heap(k: i32) -> bool {
    matches!(k, 12 | 13 | 63) || is_array(k) || is_reference(k)
}
pub fn element_kind(k: i32) -> Result<i32> {
    Ok(match k {
        14 => 10,
        15 => 11,
        16 => 12,
        17 => 13,
        30 => 27,
        50 => 47,
        58 => 55,
        66 => 63,
        _ => anyhow::bail!("System4 unknown array type {k}"),
    })
}
