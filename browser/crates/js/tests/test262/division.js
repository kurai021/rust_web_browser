// Copyright 2009 the Sputnik authors. All rights reserved.
// Governed by the BSD license in LICENSE.
// Upstream language/expressions/division/S11.5.2_A2.1_T1.js
if (1 / 1 !== 1) { throw new Test262Error('#1: GetValue division'); }
var x = 1;
if (x / 1 !== 1) { throw new Test262Error('#2: GetValue division'); }
var y = 1;
if (1 / y !== 1) { throw new Test262Error('#3: GetValue division'); }
var x = 1;
var y = 1;
if (x / y !== 1) { throw new Test262Error('#4: GetValue division'); }
var objectx = new Object();
var objecty = new Object();
objectx.prop = 1;
objecty.prop = 1;
if (objectx.prop / objecty.prop !== 1) { throw new Test262Error('#5: GetValue division'); }
