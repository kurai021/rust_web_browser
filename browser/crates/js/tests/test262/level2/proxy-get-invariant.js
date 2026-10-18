// Copyright (C) 2015 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Proxy/get/not-same-value-configurable-false-writable-false-throws.js
var target = {};
var p = new Proxy(target, {get: function() { return 2; }});
Object.defineProperty(target, 'attr', {configurable: false, writable: false, value: 1});
assert.throws(TypeError, function() { p.attr; });
assert.throws(TypeError, function() { p['attr']; });
