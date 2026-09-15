//! Retain our OWN copy of an inherited CTM for the complete atomic saved-state
//! lifetime. The original ID may have only a KMS property ref and disappear as
//! soon as a compositor changes CTM. Identical bytes under another unretained ID
//! do not make that saved ID valid again.
use super::DrmDeviceFd;
use drm::control::Device;
use std::io;

pub(super) trait BlobIo {
    fn copy_blob(&self, original: u64) -> io::Result<u64>;
    fn destroy_blob(&self, owned: u64);
}

impl BlobIo for DrmDeviceFd {
    fn copy_blob(&self, original: u64) -> io::Result<u64> {
        let bytes: [u8; 72] = self
            .get_property_blob(original)?
            .try_into()
            .map_err(|_| io::Error::other("invalid saved CTM size"))?;
        self.create_property_blob(&bytes)?
            .as_blob()
            .ok_or_else(|| io::Error::other("invalid saved CTM blob"))
    }
    fn destroy_blob(&self, owned: u64) {
        let _ = self.destroy_property_blob(owned);
    }
}

#[derive(Debug)]
pub(super) struct SavedCtm<I: BlobIo> {
    io: I,
    pub(super) blob: u64,
}
impl<I: BlobIo> SavedCtm<I> {
    pub(super) fn new(io: I, original: u64) -> io::Result<Self> {
        let blob = if original == 0 { 0 } else { io.copy_blob(original)? };
        Ok(Self { io, blob })
    }
}
impl<I: BlobIo> Drop for SavedCtm<I> {
    fn drop(&mut self) {
        if self.blob != 0 {
            self.io.destroy_blob(self.blob);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};
    #[derive(Clone)]
    struct Fake(Rc<RefCell<HashMap<u64, [u8; 72]>>>);
    impl BlobIo for Fake {
        fn copy_blob(&self, original: u64) -> io::Result<u64> {
            let bytes = self.0.borrow()[&original];
            self.0.borrow_mut().insert(2, bytes);
            Ok(2)
        }
        fn destroy_blob(&self, owned: u64) {
            self.0.borrow_mut().remove(&owned);
        }
    }
    #[test]
    fn saved_ctm_survives_original_id_destruction_and_neutral_recreation() {
        let io = Fake(Rc::new(RefCell::new(HashMap::from([(1, [17; 72])]))));
        let saved = SavedCtm::new(io.clone(), 1).unwrap();
        // KMS drops the last inherited reference when nightlight replaces CTM.
        io.0.borrow_mut().remove(&1);
        // Nightlight turns off with identical contents, but a DIFFERENT ID.
        io.0.borrow_mut().insert(3, [17; 72]);
        assert!(!io.0.borrow().contains_key(&1));
        assert_eq!(io.0.borrow()[&saved.blob], [17; 72]);
        assert_ne!(saved.blob, 3);
        // The atomic restore request must use saved.blob, never inherited 1.
        drop(saved);
        assert!(!io.0.borrow().contains_key(&2));
        assert!(io.0.borrow().contains_key(&3)); // Never destroy another owner.
        let zero = SavedCtm::new(io, 0).unwrap();
        assert_eq!(zero.blob, 0);
    }
}
