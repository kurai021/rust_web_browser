// Copyright (C) 2015 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Reflect/get/return-value-from-receiver.js
var o1 = {};
var receiver = {y: 42};
Object.defineProperty(o1, 'x', {get: function() { return this.y; }});
assert.sameValue(Reflect.get(o1, 'x', receiver), 42, 'Return own property value using a receiver');
var o2 = Object.create(o1);
assert.sameValue(Reflect.get(o2, 'x', receiver), 42, 'Return prototype property value using a receiver');
