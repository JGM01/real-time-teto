//! Bounded g2p_en inference from the original NumPy checkpoint.
use crate::{
    error::{Result, SpeechError},
    limits::*,
    types::EnglishPhone,
};
use ndarray::{Array2, ArrayView1, ArrayViewMut1, linalg::general_mat_vec_mul};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
const HIDDEN: usize = 256;
const GATES: usize = 3 * HIDDEN;
const CHECKPOINT: [u8; 32] = [
    0xb8, 0xaf, 0x35, 0xe4, 0x59, 0x6d, 0x8d, 0xd5, 0x83, 0x6d, 0xfd, 0x3f, 0xe9, 0xb2, 0xba, 0x4f,
    0x97, 0xb9, 0xc3, 0x11, 0xef, 0xe8, 0x87, 0x95, 0x44, 0xcb, 0xcf, 0xcb, 0xd5, 0x66, 0xd8, 0xc6,
];
pub const VOCABULARY: [&str; 74] = [
    "<pad>", "<unk>", "<s>", "</s>", "AA0", "AA1", "AA2", "AE0", "AE1", "AE2", "AH0", "AH1", "AH2",
    "AO0", "AO1", "AO2", "AW0", "AW1", "AW2", "AY0", "AY1", "AY2", "B", "CH", "D", "DH", "EH0",
    "EH1", "EH2", "ER0", "ER1", "ER2", "EY0", "EY1", "EY2", "F", "G", "HH", "IH0", "IH1", "IH2",
    "IY0", "IY1", "IY2", "JH", "K", "L", "M", "N", "NG", "OW0", "OW1", "OW2", "OY0", "OY1", "OY2",
    "P", "R", "S", "SH", "T", "TH", "UH0", "UH1", "UH2", "UW", "UW0", "UW1", "UW2", "V", "W", "Y",
    "Z", "ZH",
];
struct Cell {
    input: Array2<f32>,
    recurrent: Array2<f32>,
    input_bias: Vec<f32>,
    recurrent_bias: Vec<f32>,
}
pub struct G2pEnGru {
    encoder_embedding: Array2<f32>,
    decoder_embedding: Array2<f32>,
    encoder: Cell,
    decoder: Cell,
    projection: Array2<f32>,
    projection_bias: Vec<f32>,
    hidden: [f32; HIDDEN],
    input_gate: [f32; GATES],
    recurrent_gate: [f32; GATES],
    logits: [f32; 74],
    tokens: [u8; MAX_G2P_STEPS],
}
impl G2pEnGru {
    pub fn load(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        if file.metadata()?.len() > MAX_MODEL_BYTES {
            return Err(SpeechError::Bounds);
        }
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        let mut total = 0u64;
        loop {
            let n = file.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > MAX_MODEL_BYTES {
                return Err(SpeechError::Bounds);
            }
            hash.update(&bytes[..n]);
        }
        if <[u8; 32]>::from(hash.finalize()) != CHECKPOINT {
            return Err(SpeechError::ModelMismatch);
        }
        file.seek(SeekFrom::Start(0))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|_| SpeechError::ModelMismatch)?;
        if zip.len() != 12 {
            return Err(SpeechError::ModelMismatch);
        }
        Ok(Self {
            encoder_embedding: matrix(&mut zip, "enc_emb", 29, HIDDEN)?,
            decoder_embedding: matrix(&mut zip, "dec_emb", 74, HIDDEN)?,
            encoder: cell(&mut zip, "enc")?,
            decoder: cell(&mut zip, "dec")?,
            projection: matrix(&mut zip, "fc_w", 74, HIDDEN)?,
            projection_bias: tensor(&mut zip, "fc_b", &[74])?,
            hidden: [0.; HIDDEN],
            input_gate: [0.; GATES],
            recurrent_gate: [0.; GATES],
            logits: [0.; 74],
            tokens: [0; MAX_G2P_STEPS],
        })
    }
    pub fn revision(&self) -> [u8; 32] {
        CHECKPOINT
    }
    pub fn predict_into(&mut self, word: &str, output: &mut Vec<EnglishPhone>) -> Result<()> {
        let tokens = self.decode_into(word)?;
        if output.len() + tokens.len() > MAX_PHONES {
            return Err(SpeechError::Bounds);
        }
        for &token in tokens {
            output.push(EnglishPhone::parse(VOCABULARY[token as usize])?);
        }
        Ok(())
    }
    pub fn decode(&mut self, word: &str) -> Result<Vec<u8>> {
        self.decode_into(word).map(Vec::from)
    }
    fn decode_into(&mut self, word: &str) -> Result<&[u8]> {
        if word.is_empty()
            || word.len() > MAX_WORD_ASCII
            || !word.is_ascii()
            || word.bytes().any(|b| b.is_ascii_control())
        {
            return Err(SpeechError::Pronunciation);
        }
        self.hidden.fill(0.);
        for byte in word.bytes().chain(std::iter::once(0)) {
            let index = if byte == 0 {
                2
            } else if byte.is_ascii_lowercase() {
                usize::from(byte - b'a') + 3
            } else {
                1
            };
            step(
                &self.encoder,
                self.encoder_embedding
                    .row(index)
                    .as_slice()
                    .ok_or(SpeechError::ModelMismatch)?,
                &mut self.hidden,
                &mut self.input_gate,
                &mut self.recurrent_gate,
            );
        }
        let mut token = 2;
        for token_count in 0..MAX_G2P_STEPS {
            step(
                &self.decoder,
                self.decoder_embedding
                    .row(token)
                    .as_slice()
                    .ok_or(SpeechError::ModelMismatch)?,
                &mut self.hidden,
                &mut self.input_gate,
                &mut self.recurrent_gate,
            );
            general_mat_vec_mul(
                1.,
                &self.projection,
                &ArrayView1::from(&self.hidden),
                0.,
                &mut ArrayViewMut1::from(&mut self.logits),
            );
            for (value, bias) in self.logits.iter_mut().zip(&self.projection_bias) {
                *value += bias;
            }
            token = greedy(&self.logits)?;
            if token == 3 {
                if token_count == 0 {
                    return Err(SpeechError::Pronunciation);
                }
                return Ok(&self.tokens[..token_count]);
            }
            if token < 4 {
                return Err(SpeechError::Pronunciation);
            }
            self.tokens[token_count] = token as u8;
        }
        Err(SpeechError::DecoderLimit)
    }
}
fn greedy(logits: &[f32]) -> Result<usize> {
    let mut best = 0;
    for (i, value) in logits.iter().enumerate() {
        if !value.is_finite() {
            return Err(SpeechError::ModelMismatch);
        }
        if *value > logits[best] {
            best = i;
        }
    }
    Ok(best)
}
fn step(
    cell: &Cell,
    input: &[f32],
    hidden: &mut [f32; HIDDEN],
    input_gate: &mut [f32; GATES],
    recurrent_gate: &mut [f32; GATES],
) {
    general_mat_vec_mul(
        1.,
        &cell.input,
        &ArrayView1::from(input),
        0.,
        &mut ArrayViewMut1::from(&mut *input_gate),
    );
    general_mat_vec_mul(
        1.,
        &cell.recurrent,
        &ArrayView1::from(&*hidden),
        0.,
        &mut ArrayViewMut1::from(&mut *recurrent_gate),
    );
    for i in 0..GATES {
        input_gate[i] += cell.input_bias[i];
        recurrent_gate[i] += cell.recurrent_bias[i];
    }
    for i in 0..HIDDEN {
        let reset = 1. / (1. + (-(input_gate[i] + recurrent_gate[i])).exp());
        let update = 1. / (1. + (-(input_gate[HIDDEN + i] + recurrent_gate[HIDDEN + i])).exp());
        let new = (input_gate[2 * HIDDEN + i] + reset * recurrent_gate[2 * HIDDEN + i]).tanh();
        hidden[i] = (1. - update) * new + update * hidden[i];
    }
}
fn cell(zip: &mut zip::ZipArchive<File>, prefix: &str) -> Result<Cell> {
    Ok(Cell {
        input: matrix(zip, &format!("{prefix}_w_ih"), GATES, HIDDEN)?,
        recurrent: matrix(zip, &format!("{prefix}_w_hh"), GATES, HIDDEN)?,
        input_bias: tensor(zip, &format!("{prefix}_b_ih"), &[GATES as u64])?,
        recurrent_bias: tensor(zip, &format!("{prefix}_b_hh"), &[GATES as u64])?,
    })
}
fn matrix(
    zip: &mut zip::ZipArchive<File>,
    name: &str,
    rows: usize,
    columns: usize,
) -> Result<Array2<f32>> {
    Array2::from_shape_vec(
        (rows, columns),
        tensor(zip, name, &[rows as u64, columns as u64])?,
    )
    .map_err(|_| SpeechError::ModelMismatch)
}
fn tensor(zip: &mut zip::ZipArchive<File>, name: &str, shape: &[u64]) -> Result<Vec<f32>> {
    let mut entry = zip
        .by_name(&format!("{name}.npy"))
        .map_err(|_| SpeechError::ModelMismatch)?;
    let count = shape.iter().product::<u64>();
    if entry.size() > count * 4 + 2048 {
        return Err(SpeechError::Bounds);
    }
    let mut prefix = [0u8; 10];
    entry.read_exact(&mut prefix)?;
    if &prefix[..8] != b"\x93NUMPY\x01\x00" || u16::from_le_bytes([prefix[8], prefix[9]]) > 1024 {
        return Err(SpeechError::ModelMismatch);
    }
    let reader = std::io::Cursor::new(prefix).chain(entry);
    let npy = npyz::NpyFile::new(reader).map_err(|_| SpeechError::ModelMismatch)?;
    if npy.shape() != shape || npy.order() != npyz::Order::C {
        return Err(SpeechError::ModelMismatch);
    }
    let data = npy.data::<f32>().map_err(|_| SpeechError::ModelMismatch)?;
    let mut values = Vec::with_capacity(count as usize);
    for value in data {
        let value = value.map_err(|_| SpeechError::ModelMismatch)?;
        if !value.is_finite() {
            return Err(SpeechError::ModelMismatch);
        }
        values.push(value);
    }
    if values.len() != count as usize {
        return Err(SpeechError::ModelMismatch);
    }
    Ok(values)
}
#[cfg(test)]
mod tests {
    #[test]
    fn greedy_ties_select_first_token() {
        assert_eq!(super::greedy(&[1., 3., 3., 0.]).unwrap(), 1);
    }
}
