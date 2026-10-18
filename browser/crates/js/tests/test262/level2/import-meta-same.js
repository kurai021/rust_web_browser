// Copyright (C) 2018 André Bargull. All rights reserved.
// BSD-3-Clause; ../LICENSE. Upstream: language/expressions/import.meta/same-object-returned.js
var a = import.meta;
var b = function() { return import.meta; }();
assert.sameValue(import.meta, a, "import.meta accessed directly and accessed via variable declaration");
assert.sameValue(import.meta, b, "import.meta accessed directly and accessed via function return value");
