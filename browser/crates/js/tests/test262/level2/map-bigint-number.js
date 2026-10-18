// Copyright (C) 2021 Rick Waldron. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Map/bigint-number-same-value.js
const number = 9007199254740991;
const bigint = 9007199254740991n;
const m = new Map([[number, number], [bigint, bigint]]);
assert.sameValue(m.size, 2);
assert.sameValue(m.has(number), true);
assert.sameValue(m.has(bigint), true);
assert.sameValue(m.get(number), number);
assert.sameValue(m.get(bigint), bigint);
m.delete(number);
assert.sameValue(m.size, 1);
assert.sameValue(m.has(number), false);
m.delete(bigint);
assert.sameValue(m.size, 0);
assert.sameValue(m.has(bigint), false);
m.set(number, number);
assert.sameValue(m.size, 1);
m.set(bigint, bigint);
assert.sameValue(m.size, 2);
