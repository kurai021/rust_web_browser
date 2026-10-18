// Copyright (C) 2017 Caio Lima. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/BigInt/constructor-empty-string.js
assert.sameValue(BigInt(""), 0n);
assert.sameValue(BigInt(" "), 0n);
