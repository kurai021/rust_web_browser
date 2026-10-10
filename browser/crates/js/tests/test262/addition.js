// Copyright 2009 the Sputnik authors. All rights reserved.
// Governed by the BSD license in LICENSE.
// Upstream language/expressions/addition/S11.6.1_A2.1_T1.js
if (1 + 1 !== 2) { throw new Test262Error('#1: GetValue addition'); }
var x = 1;
if (x + 1 !== 2) { throw new Test262Error('#2: GetValue addition'); }
var y = 1;
if (1 + y !== 2) { throw new Test262Error('#3: GetValue addition'); }
var x = 1;
var y = 1;
if (x + y !== 2) { throw new Test262Error('#4: GetValue addition'); }
var objectx = new Object();
var objecty = new Object();
objectx.prop = 1;
objecty.prop = 1;
if (objectx.prop + objecty.prop !== 2) { throw new Test262Error('#5: GetValue addition'); }
