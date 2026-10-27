# teto-speech

English speech contracts for the Teto pipeline. The default build contains
types and traits only. Model, voicebank and audio backends are optional
features.

The first boundary is `SpeechInput -> PhonemeSequence`. The next boundary is
`PhonemeSequence -> UnitPlan`. `UnitPlan` refers to OTO aliases and timing in a
versioned voicebank manifest.

The current backends are `todo!()` seams:

- CMUdict phonemization and the g2p_en OOV network.
- Syllable segmentation and ARPAbet to X-SAMPA mapping.
- Teto English OTO import and CVVC unit selection.
- Rule-based prosody.
- PSOLA rendering and device output.

This crate is not a working TTS yet. The Teto voicebank is user-supplied and is
not distributed. The code is MIT OR Apache-2.0; that license does not cover the
recordings.
