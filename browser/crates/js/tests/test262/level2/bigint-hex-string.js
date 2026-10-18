// Copyright (C) 2017 Caio Lima. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/BigInt/constructor-from-hex-string.js
assert.sameValue(BigInt("0xa"), 10n);
assert.sameValue(BigInt("0xff"), 255n);
assert.sameValue(BigInt("0xfabc"), 64188n);
assert.sameValue(BigInt("0xfffffffffffffffffff"), 75557863725914323419135n);
assert.sameValue(BigInt("0Xa"), 10n);
assert.sameValue(BigInt("0Xff"), 255n);
assert.sameValue(BigInt("0Xfabc"), 64188n);
assert.sameValue(BigInt("0Xfffffffffffffffffff"), 75557863725914323419135n);
