//! English word lookup and bounded OOV prediction.
#[cfg(feature = "g2p-gru")]
pub use super::gru::G2pEnGru;
#[cfg(feature = "cmudict")]
mod frontend;
#[cfg(feature = "cmudict")]
pub use frontend::CmudictPhonemizer;

pub enum OovBackend {
    Disabled,
    #[cfg(feature = "g2p-gru")]
    Gru(Box<G2pEnGru>),
}
