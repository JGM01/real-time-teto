# Third-party speech sources

The project code uses MIT OR Apache-2.0. These sources retain their own notices.

- **CMUdict**: direct data from `cmusphinx/cmudict`, revision
  `74790861f652b15e4ac49015a90074ad62a27690`. The dictionary and its permissive
  license are in `teto-speech/assets/cmudict/`. No Python dictionary wrapper is used.
- **g2p_en**: numerical inference follows `Kyubyong/g2p`, revision
  `c6439c274c42b9724a7fee1dc07ca6a4c68a0538`, by Kyubyong Park and Jongseok Kim.
  Apache-2.0 notice: `teto-speech/assets/licenses/g2p-APACHE-2.0.txt`.
  The checkpoint remains a separately prepared local asset.
- **OpenUtau**: English X-SAMPA syllable and ending candidate order follows
  `EnXSampaPhonemizer.cs` and `SyllableBasedPhonemizer.cs` at revision
  `b7c35b90941695074ead1063037099f81cd588e0`. Copyright (c) 2020 OpenUtau.
  MIT notice: `teto-speech/assets/licenses/OpenUtau-MIT.txt`.
  The development oracle builds the pinned upstream projects.

`teto-speech/assets/p3-sources.json` records source and corpus hashes.
The Teto recordings have separate terms. This repository distributes no recordings,
derived voicebank packs, or golden Teto audio.
