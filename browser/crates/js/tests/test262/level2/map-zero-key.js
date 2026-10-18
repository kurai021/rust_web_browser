// Copyright (C) 2015 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Map/prototype/set/replaces-a-value-normalizes-zero-key.js
var map = new Map([[+0, 1]]);
map.set(-0, 42);
assert.sameValue(map.get(+0), 42, 'zero key is normalized in SameValueZero');
map = new Map([[-0, 1]]);
map.set(+0, 42);
assert.sameValue(map.get(-0), 42, 'zero key is normalized in SameValueZero');
