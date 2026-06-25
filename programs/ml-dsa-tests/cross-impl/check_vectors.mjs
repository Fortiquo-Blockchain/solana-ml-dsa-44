// check_vectors.mjs — cross-implementation half of the ML-DSA-44 conformance suite.
//
// Proves the JavaScript wallet stack (@noble/post-quantum, the library the sibling
// `../ml-dsa-44/` sample uses) agrees with the same NIST FIPS 204 answer key that
// the Rust validator is pinned to in `../tests/fips204_vectors.rs`:
//
//   1. KeyGen — @noble's deterministic keygen(seed) reproduces NIST's public AND
//      secret key, byte-for-byte, for every vector in the shared fixture.
//   2. External interop — @noble produces a DETERMINISTIC external (empty-context)
//      signature over a fixed message and writes it to
//      `../tests/vectors/cross_impl_external.json`. The Rust test then verifies
//      that signature through the validator's real verify path and reproduces it
//      byte-for-byte. This is the exact path a Phase-1 wallet uses.
//
// Run it with Windows Node (there is no Node inside WSL):
//   node programs/ml-dsa-tests/cross-impl/check_vectors.mjs
// It resolves @noble from the sibling sample's node_modules. Override with:
//   NOBLE_MLDSA=/abs/path/to/@noble/post-quantum/ml-dsa.js node .../check_vectors.mjs

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));

// Default: the sibling `ml-dsa-44` sample that ships @noble/post-quantum.
const noblePath = process.env.NOBLE_MLDSA
  ? resolve(process.env.NOBLE_MLDSA)
  : resolve(here, '../../../../ml-dsa-44/node_modules/@noble/post-quantum/ml-dsa.js');

let ml_dsa44;
try {
  ({ ml_dsa44 } = await import(pathToFileURL(noblePath).href));
} catch (e) {
  // Only show the resolution hint when the module truly isn't found — otherwise a
  // syntax/runtime error inside a resolved ml-dsa.js would be misattributed.
  if (e && e.code === 'ERR_MODULE_NOT_FOUND') {
    console.error(
      `Could not resolve @noble ml-dsa at:\n  ${noblePath}\n` +
        `Set NOBLE_MLDSA to the path of @noble/post-quantum/ml-dsa.js (see README.md).`,
    );
  }
  throw e;
}

const hexToBytes = (h) => Uint8Array.from(Buffer.from(h, 'hex'));
const bytesToHex = (b) => Buffer.from(b).toString('hex');
const utf8 = (s) => new TextEncoder().encode(s);
const eq = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

let checks = 0;
function assert(cond, msg) {
  if (!cond) {
    console.error('FAIL: ' + msg);
    process.exit(1);
  }
  checks++;
}

const fixturePath = resolve(here, '../tests/vectors/ml_dsa_44_kat.json');
const kat = JSON.parse(readFileSync(fixturePath, 'utf8'));

const nonEmpty = (a, name) => {
  assert(Array.isArray(a) && a.length > 0, `fixture ${name} must be a non-empty array`);
  return a;
};

// 1. KeyGen — NIST gold on @noble (deterministic from the 32-byte seed).
for (const t of nonEmpty(kat.keygen, 'keygen')) {
  const { publicKey, secretKey } = ml_dsa44.keygen(hexToBytes(t.seed));
  assert(eq(publicKey, hexToBytes(t.pk)), `keygen pk tcId ${t.tcId}`);
  assert(eq(secretKey, hexToBytes(t.sk)), `keygen sk tcId ${t.tcId}`);
}
console.log(`[ok] @noble keygen matches NIST ACVP for ${kat.keygen.length} vectors`);

// 1b. Internal sign/verify — NIST gold on @noble's signing core, at parity with the
//     Rust side. ACVP sigGen/sigVer are the internal interface (raw message);
//     deterministic signing (extraEntropy:false => rnd=0) must reproduce gold bytes.
for (const t of nonEmpty(kat.siggen_det, 'siggen_det')) {
  const sig = ml_dsa44.internal.sign(hexToBytes(t.message), hexToBytes(t.sk), { extraEntropy: false });
  assert(eq(sig, hexToBytes(t.signature)), `internal sign tcId ${t.tcId}`);
}
for (const t of nonEmpty(kat.sigver, 'sigver')) {
  const ok = ml_dsa44.internal.verify(hexToBytes(t.signature), hexToBytes(t.message), hexToBytes(t.pk));
  assert(ok === t.testPassed, `internal verify tcId ${t.tcId} (${t.reason})`);
}
console.log(
  `[ok] @noble internal sign/verify match NIST ACVP (${kat.siggen_det.length} sign, ${kat.sigver.length} verify)`,
);

// 2. External empty-context interop — the path Phase-1 wallets actually use.
const xi = kat.external_interop;
assert(xi && typeof xi === 'object', 'fixture external_interop block missing');
for (const f of ['seed', 'pk', 'message_utf8']) {
  assert(typeof xi[f] === 'string' && xi[f].length > 0, `external_interop.${f} missing/empty`);
}
const { publicKey, secretKey } = ml_dsa44.keygen(hexToBytes(xi.seed));
assert(eq(publicKey, hexToBytes(xi.pk)), 'external_interop seed -> pk');

const message = utf8(xi.message_utf8);
// extraEntropy:false disables the hedged randomizer -> deterministic (rnd = 0).
const extSig = ml_dsa44.sign(message, secretKey, { extraEntropy: false });
assert(ml_dsa44.verify(extSig, message, publicKey), 'self-verify external signature');
assert(!ml_dsa44.verify(extSig, utf8(xi.message_utf8 + '!'), publicKey), 'tampered must fail');
const extSig2 = ml_dsa44.sign(message, secretKey, { extraEntropy: false });
assert(eq(extSig, extSig2), 'external deterministic sign is reproducible');

const outPath = resolve(here, '../tests/vectors/cross_impl_external.json');
writeFileSync(
  outPath,
  JSON.stringify(
    {
      _comment:
        'Deterministic ML-DSA-44 EXTERNAL (empty-context) signature from @noble/post-quantum, ' +
        'consumed by tests/fips204_vectors.rs to prove wallet(JS)->validator(Rust) interop. ' +
        'Regenerate: node programs/ml-dsa-tests/cross-impl/check_vectors.mjs',
      _source: '@noble/post-quantum ml_dsa44.sign(msg, sk, {extraEntropy:false})',
      message_utf8: xi.message_utf8,
      ext_sig_det: bytesToHex(extSig),
    },
    null,
    1,
  ) + '\n',
);
console.log(`[ok] wrote external interop artifact -> ${outPath}`);

// Floor on assertions actually run, so a thin/missing fixture can't report PASS over nothing.
const floor = kat.keygen.length * 2 + kat.siggen_det.length + kat.sigver.length;
assert(checks >= floor, `too few assertions ran (${checks} < ${floor}) — fixture under-populated?`);
console.log(`PASS — ${checks} assertions (@noble <-> NIST keygen/sign/verify + external interop)`);
