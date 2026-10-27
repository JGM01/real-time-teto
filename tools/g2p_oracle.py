#!/usr/bin/env python3
"""Run the pinned g2p_en NumPy decoder without its text frontend."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
PINS = json.loads((ROOT / "teto-speech/assets/p3-sources.json").read_text())
CHECKPOINT_SHA256 = next(
    record["sha256"]
    for record in PINS["g2p"]["files"]
    if record["path"].endswith("checkpoint20.npz")
)


def cell(x, h, weights, prefix):
    """Apply the upstream reset, update, and candidate equations."""
    a = np.matmul(x, weights[prefix + "_w_ih"].T) + weights[prefix + "_b_ih"]
    b = np.matmul(h, weights[prefix + "_w_hh"].T) + weights[prefix + "_b_hh"]
    width = h.shape[-1]
    gates = 1 / (1 + np.exp(-(a[:, :2 * width] + b[:, :2 * width])))
    reset, update = np.split(gates, 2, -1)
    candidate = np.tanh(a[:, 2 * width:] + reset * b[:, 2 * width:])
    return (1 - update) * candidate + update * h


def predict(word, weights):
    graphemes = {c: i + 3 for i, c in enumerate("abcdefghijklmnopqrstuvwxyz")}
    hidden = np.zeros((1, weights["enc_w_hh"].shape[1]), np.float32)
    for char in list(word) + ["</s>"]:
        token = 2 if char == "</s>" else graphemes.get(char, 1)
        hidden = cell(weights["enc_emb"][[token]], hidden, weights, "enc")
    tokens = []
    token = 2
    for _ in range(20):
        hidden = cell(weights["dec_emb"][[token]], hidden, weights, "dec")
        logits = np.matmul(hidden, weights["fc_w"].T) + weights["fc_b"]
        token = int(logits.argmax())
        if token == 3:
            return {"word": word, "tokens": tokens, "eos": True}
        tokens.append(token)
    return {"word": word, "tokens": tokens, "eos": False}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("checkpoint", type=Path)
    parser.add_argument("words", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if hashlib.sha256(args.checkpoint.read_bytes()).hexdigest() != CHECKPOINT_SHA256:
        raise ValueError("checkpoint hash mismatch")
    weights = np.load(args.checkpoint, allow_pickle=False)
    words = args.words.read_text().splitlines()
    output = {"checkpoint_sha256": CHECKPOINT_SHA256, "numpy": np.__version__,
              "records": [predict(word, weights) for word in words]}
    args.output.write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
