//! Bank-local X-SAMPA symbols and bounded alias segmentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BankSymbol {
    A,
    Ae,
    Ah,
    O,
    Aw,
    Ay,
    B,
    Ch,
    D,
    Dh,
    E,
    Er,
    Ey,
    F,
    G,
    H,
    I,
    Iy,
    Jh,
    K,
    L,
    M,
    N,
    Ng,
    Ow,
    Oy,
    P,
    R,
    S,
    Sh,
    T,
    Th,
    U,
    Uw,
    V,
    W,
    Y,
    Z,
    Zh,
    Schwa,
    AShort,
    EShort,
    OShort,
    Boundary,
}
impl BankSymbol {
    pub fn parts(&self) -> &[Self] {
        use BankSymbol::*;
        match self {
            Ch => &[T, Sh],
            Jh => &[D, Zh],
            Ay => &[AShort, I],
            Aw => &[AShort, U],
            Ey => &[EShort, I],
            Ow => &[OShort, U],
            Oy => &[O, I],
            _ => std::slice::from_ref(self),
        }
    }
    pub fn text(self) -> &'static str {
        use BankSymbol::*;
        match self {
            A => "A",
            Ae => "{",
            Ah => "V",
            O => "O",
            Aw => "aU",
            Ay => "aI",
            B => "b",
            Ch => "tS",
            D => "d",
            Dh => "D",
            E => "E",
            Er => "3",
            Ey => "eI",
            F => "f",
            G => "g",
            H => "h",
            I => "I",
            Iy => "i",
            Jh => "dZ",
            K => "k",
            L => "l",
            M => "m",
            N => "n",
            Ng => "N",
            Ow => "oU",
            Oy => "OI",
            P => "p",
            R => "r",
            S => "s",
            Sh => "S",
            T => "t",
            Th => "T",
            U => "U",
            Uw => "u",
            V => "v",
            W => "w",
            Y => "j",
            Z => "z",
            Zh => "Z",
            Schwa => "@",
            AShort => "a",
            EShort => "e",
            OShort => "o",
            Boundary => "-",
        }
    }
    #[cfg(any(
        feature = "voicebank-wav",
        feature = "testing",
        all(test, feature = "cmudict")
    ))]
    pub(crate) fn parse_alias(mut alias: &str, out: &mut Vec<Self>) -> bool {
        use BankSymbol::*;
        const SYMBOLS: [BankSymbol; 43] = [
            Aw, Ay, Ch, Ey, Jh, Ow, Oy, A, Ae, Ah, O, B, D, Dh, E, Er, F, G, H, I, Iy, K, L, M, N,
            Ng, P, R, S, Sh, T, Th, U, Uw, V, W, Y, Z, Zh, Schwa, AShort, EShort, OShort,
        ];
        while !alias.is_empty() {
            if let Some((_, vowel)) = [
                ("@5", Schwa),
                ("u5", Uw),
                ("O5", O),
                ("A5", A),
                ("E5", E),
                ("I5", I),
                ("i5", Iy),
            ]
            .iter()
            .find(|(text, _)| alias.starts_with(text))
            {
                out.push(*vowel);
                out.push(L);
                alias = &alias[2..];
                continue;
            }
            if alias.starts_with([' ', '_']) {
                alias = &alias[1..];
                continue;
            }
            if alias.starts_with('-') {
                out.push(Boundary);
                alias = &alias[1..];
                continue;
            }
            let Some(symbol) = SYMBOLS.iter().find(|s| alias.starts_with(s.text())) else {
                return false;
            };
            out.push(*symbol);
            alias = &alias[symbol.text().len()..];
        }
        true
    }
}
