use crate::{de::Deserializer, Asn1DerError, Result};
use serde::de::{DeserializeSeed, SeqAccess};

/// A deserializer for sequences
pub struct Sequence<'a, 'de> {
    de: &'a mut Deserializer<'de>,
    len: usize,
}
impl<'a, 'de> Sequence<'a, 'de> {
    /// Creates a lazy deserializer that can walk through the sequence's sub-elements
    pub fn deserialize_lazy(de: &'a mut Deserializer<'de>, len: usize) -> Self {
        Self { de, len }
    }
}
impl<'a, 'de> SeqAccess<'de> for Sequence<'a, 'de> {
    type Error = Asn1DerError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        // Check if there are still some data remaining
        if self.len == 0 {
            return Ok(None);
        }

        // Deserialize the element
        let pos = self.de.reader.pos();
        let element = seed.deserialize(&mut *self.de)?;
        self.len -= self.de.reader.pos() - pos;

        Ok(Some(element))
    }
}
