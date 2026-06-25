#!/usr/bin/env python3
"""Regenerate tests/vectors/ml_dsa_44_kat.json from the NIST ACVP FIPS 204 vectors.

The authoritative vectors are vendored inside the `fips204` crate we depend on:
    fips204-0.4.6/tests/nist_vectors/{ML-DSA-keyGen,ML-DSA-sigGen,ML-DSA-sigVer}-FIPS204/internalProjection.json
which are themselves copied from
    https://github.com/usnistgov/ACVP-Server/tree/65370b861b96efd30dfe0daae607bde26a78a5c8/gen-val/json-files

This script extracts a small ML-DSA-44 excerpt and writes the committed fixture.
It is provenance/reproducibility only — the generated JSON is what the tests read.

Usage (from a WSL shell, after `cargo fetch` has populated the registry, or by
unpacking the cached .crate):

    # point at the vendored vectors (a fips204 source tree):
    NIST_DIR=~/fips204_inspect/fips204-0.4.6/tests/nist_vectors \
        python3 programs/ml-dsa-tests/cross-impl/gen_fixture.py

    # to obtain that tree:
    CR=$(ls ~/.cargo/registry/cache/*/fips204-0.4.6.crate | head -1)
    mkdir -p ~/fips204_inspect && tar -xzf "$CR" -C ~/fips204_inspect
"""
import json
import os
import sys

NIST_DIR = os.environ.get("NIST_DIR")
if not NIST_DIR or not os.path.isdir(NIST_DIR):
    sys.exit(
        "set NIST_DIR to a fips204 .../tests/nist_vectors directory (see this file's docstring)"
    )

OUT = os.path.join(os.path.dirname(__file__), "..", "tests", "vectors", "ml_dsa_44_kat.json")


def load(name):
    path = os.path.join(NIST_DIR, name, "internalProjection.json")
    try:
        with open(path) as f:
            return json.load(f)
    except FileNotFoundError:
        sys.exit(f"missing NIST vector file {path}; is NIST_DIR a fips204 tests/nist_vectors tree?")


def first44(v, name, pred=lambda tg: True):
    groups = [tg for tg in v["testGroups"] if tg.get("parameterSet") == "ML-DSA-44" and pred(tg)]
    if not groups:
        sys.exit(f"no ML-DSA-44 test group found in {name}")
    return groups[0]


kg = load("ML-DSA-keyGen-FIPS204")
sg = load("ML-DSA-sigGen-FIPS204")
sv = load("ML-DSA-sigVer-FIPS204")

# keyGen: first 3 {seed, pk, sk}
keygen = [
    {"tcId": t["tcId"], "seed": t["seed"], "pk": t["pk"], "sk": t["sk"]}
    for t in first44(kg, "keyGen")["tests"][:3]
]

# sigGen deterministic group (rnd = 0): 2 smallest messages {sk, message, signature}
det = sorted(
    first44(sg, "sigGen", lambda tg: tg.get("deterministic") is True)["tests"],
    key=lambda t: len(t["message"]),
)[:2]
siggen_det = [
    {"tcId": t["tcId"], "sk": t["sk"], "message": t["message"], "signature": t["signature"]}
    for t in det
]

# sigVer: 2 valid + one invalid per distinct failure class (covers the structural,
# signature-tamper, bound, and message-tamper rejection paths); pk lives at group level.
svg = first44(sv, "sigVer")
pk = svg["pk"]
passes = [t for t in svg["tests"] if t["testPassed"]][:2]
fails_by_reason = {}
for t in svg["tests"]:
    if not t["testPassed"]:
        fails_by_reason.setdefault(t.get("reason", ""), t)  # first occurrence per reason
fails = [fails_by_reason[r] for r in sorted(fails_by_reason)]  # sorted -> deterministic
sigver = [
    {
        "tcId": t["tcId"],
        "pk": pk,
        "message": t["message"],
        "signature": t["signature"],
        "testPassed": t["testPassed"],
        "reason": t.get("reason", ""),
    }
    for t in (passes + fails)
]

doc = {
    "_comment": "NIST ACVP FIPS 204 ML-DSA-44 known-answer vectors (excerpt). DO NOT EDIT BY HAND.",
    "_source": "https://github.com/usnistgov/ACVP-Server/tree/65370b861b96efd30dfe0daae607bde26a78a5c8/gen-val/json-files",
    "_via": "vendored in fips204 0.4.6 tests/nist_vectors; regenerate with programs/ml-dsa-tests/cross-impl/gen_fixture.py",
    "_interface": "keyGen=KeyGen(seed); sigGen/sigVer=internal interface (raw message), empty context",
    "keygen": keygen,
    "siggen_det": siggen_det,
    "sigver": sigver,
    "external_interop": {
        "_comment": "Rust+JS both derive sk from keygen[0].seed, sign this message with EXTERNAL empty-context deterministic signing; bytes must match and verify.",
        "seed": keygen[0]["seed"],
        "pk": keygen[0]["pk"],
        "message_utf8": "phase1 ml-dsa external interop",
    },
}

# Fail loudly if upstream shape changed and we silently extracted too little — the
# consumer tests only assert non-empty, so a thin fixture must be caught here.
assert len(keygen) == 3, f"expected 3 keygen vectors, got {len(keygen)}"
assert len(siggen_det) == 2, f"expected 2 deterministic siggen vectors, got {len(siggen_det)}"
assert len(passes) == 2, f"expected 2 valid sigver vectors, got {len(passes)}"
assert len(fails) >= 3, f"expected >=3 distinct invalid sigver reasons, got {len(fails)}: {sorted(fails_by_reason)}"

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w") as f:
    json.dump(doc, f, indent=1)
print(
    "wrote %s (keygen=%d siggen_det=%d sigver=%d: %d pass + %d fail)"
    % (os.path.relpath(OUT), len(keygen), len(siggen_det), len(sigver), len(passes), len(fails))
)
