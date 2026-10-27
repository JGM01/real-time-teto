//! ARPAbet to X-SAMPA mapping seam.
use crate::{text::XsampaMap, types::ArpaPhone};

/// Map ARPAbet symbols to the Teto English bank aliases.
pub struct TetoXsampaMap;

impl XsampaMap for TetoXsampaMap {
    fn map(&self, _phone: ArpaPhone) -> &'static str {
        todo!("map ARPAbet to X-SAMPA for the Teto English bank")
    }
}
