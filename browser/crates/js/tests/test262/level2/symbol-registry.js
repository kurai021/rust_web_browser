// Copyright (C) 2016 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Symbol/keyFor/arg-symbol-registry-hit.js
var canonical = Symbol.for('s');
assert.sameValue(Symbol.keyFor(canonical), 's');
