//! ARPAbet mapping at the bank boundary.
use crate::{types::ArpaPhone, voicebank::alias::BankSymbol};
pub struct TetoXsampaMap;
impl TetoXsampaMap {
    pub fn map(phone: ArpaPhone) -> BankSymbol {
        use ArpaPhone::*;
        match phone {
            Aa => BankSymbol::A,
            Ae => BankSymbol::Ae,
            Ah => BankSymbol::Ah,
            Ao => BankSymbol::O,
            Aw => BankSymbol::Aw,
            Ay => BankSymbol::Ay,
            B => BankSymbol::B,
            Ch => BankSymbol::Ch,
            D => BankSymbol::D,
            Dh => BankSymbol::Dh,
            Eh => BankSymbol::E,
            Er => BankSymbol::Er,
            Ey => BankSymbol::Ey,
            F => BankSymbol::F,
            G => BankSymbol::G,
            Hh => BankSymbol::H,
            Ih => BankSymbol::I,
            Iy => BankSymbol::Iy,
            Jh => BankSymbol::Jh,
            K => BankSymbol::K,
            L => BankSymbol::L,
            M => BankSymbol::M,
            N => BankSymbol::N,
            Ng => BankSymbol::Ng,
            Ow => BankSymbol::Ow,
            Oy => BankSymbol::Oy,
            P => BankSymbol::P,
            R => BankSymbol::R,
            S => BankSymbol::S,
            Sh => BankSymbol::Sh,
            T => BankSymbol::T,
            Th => BankSymbol::Th,
            Uh => BankSymbol::U,
            Uw => BankSymbol::Uw,
            V => BankSymbol::V,
            W => BankSymbol::W,
            Y => BankSymbol::Y,
            Z => BankSymbol::Z,
            Zh => BankSymbol::Zh,
        }
    }
}
