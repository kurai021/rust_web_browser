// Copyright (C) 2015 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Reflect/set/receiver-is-not-object.js
var o1 = {p: 42};
var receiver = 'receiver is a string';
var result = Reflect.set(o1, 'p', 43, receiver);
assert.sameValue(result, false, 'returns false');
assert.sameValue(o1.p, 42, 'does not set a value');
assert.sameValue(receiver.hasOwnProperty('p'), false, 'does not set a new property on receiver if it is not an object');
