// Copyright 2009 the Sputnik authors. All rights reserved.
// Governed by the BSD license in LICENSE.
// Upstream language/expressions/subtraction/S11.6.2_A2.1_T1.js
if (1 - 1 !== 0) { throw new Test262Error('#1: GetValue subtraction'); }
var x = 1;
if (x - 1 !== 0) { throw new Test262Error('#2: GetValue subtraction'); }
var y = 1;
if (1 - y !== 0) { throw new Test262Error('#3: GetValue subtraction'); }
var x = 1;
var y = 1;
if (x - y !== 0) { throw new Test262Error('#4: GetValue subtraction'); }
var objectx = new Object();
var objecty = new Object();
objectx.prop = 1;
objecty.prop = 1;
if (objectx.prop - objecty.prop !== 0) { throw new Test262Error('#5: GetValue subtraction'); }
