// Copyright (C) 2016 the V8 project authors. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: built-ins/Symbol/uniqueness.js
assert.notSameValue(Symbol(''), Symbol(''), 'empty string');
assert.notSameValue(Symbol(), Symbol(), 'undefined');
assert.notSameValue(Symbol(null), Symbol(null), 'null value');
assert.notSameValue(Symbol('x'), Symbol('x'), 'string "x"');
