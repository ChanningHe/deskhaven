# Generates an hbbs-format key pair with libsodium (via PyNaCl's raw bindings),
# replicating upstream rustdesk-server common.rs gen_sk(): crypto_sign_keypair,
# base64 (standard, padded) of the 64-byte secret and 32-byte public key,
# retrying while the public key contains '/' or ':'.
#
# The committed fixture in this directory was produced once with:
#   nix-shell -p 'python3.withPackages (ps: [ ps.pynacl ])' \
#     --run "python3 -I gen_fixture.py <this directory>"
# (PyNaCl 1.6.2 over the system libsodium). Re-running yields a different,
# equally valid key pair; tests only rely on the files being self-consistent.
import base64
import sys

from nacl.bindings import crypto_sign, crypto_sign_keypair

out = sys.argv[1]
for _ in range(300):
    pk, sk = crypto_sign_keypair()
    pk_b64 = base64.b64encode(pk).decode()
    if "/" not in pk_b64 and ":" not in pk_b64:
        break
sk_b64 = base64.b64encode(sk).decode()
msg = b"deskhaven M0-06 libsodium fixture"
sig = crypto_sign(msg, sk)[:64]
with open(f"{out}/id_ed25519", "w") as f:
    f.write(sk_b64)
with open(f"{out}/id_ed25519.pub", "w") as f:
    f.write(pk_b64)
with open(f"{out}/message.txt", "wb") as f:
    f.write(msg)
with open(f"{out}/signature.b64", "w") as f:
    f.write(base64.b64encode(sig).decode())
print(pk_b64)
