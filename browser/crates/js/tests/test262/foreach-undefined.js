// Copyright (c) 2012 Ecma International. All rights reserved.
// Governed by the BSD license in LICENSE.
// Upstream built-ins/Array/prototype/forEach/15.4.4.18-1-1.js
assert.throws(TypeError, function() { Array.prototype.forEach.call(undefined); });
