// Copyright 2009 the Sputnik authors. All rights reserved.
// Governed by the BSD license in LICENSE.
// Upstream language/expressions/strict-equals/S11.9.4_A1.js
if (!eval("1\u0009===\u00091")) { throw new Test262Error('#1'); }
if (!eval("1\u000B===\u000B1")) { throw new Test262Error('#2'); }
if (!eval("1\u000C===\u000C1")) { throw new Test262Error('#3'); }
if (!eval("1\u0020===\u00201")) { throw new Test262Error('#4'); }
if (!eval("1\u00A0===\u00A01")) { throw new Test262Error('#5'); }
if (!eval("1\u000A===\u000A1")) { throw new Test262Error('#6'); }
if (!eval("1\u000D===\u000D1")) { throw new Test262Error('#7'); }
if (!eval("1\u2028===\u20281")) { throw new Test262Error('#8'); }
if (!eval("1\u2029===\u20291")) { throw new Test262Error('#9'); }
if (!eval("1\u0009\u000B\u000C\u0020\u00A0\u000A\u000D\u2028\u2029===\u0009\u000B\u000C\u0020\u00A0\u000A\u000D\u2028\u20291")) { throw new Test262Error('#10'); }
