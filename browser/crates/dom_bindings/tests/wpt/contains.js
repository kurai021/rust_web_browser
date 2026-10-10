// Copyright web-platform-tests contributors. BSD-3-Clause; see LICENSE.md.
// Adapted from dom/nodes/Node-contains.html: retain the upstream ancestor oracle
// and null/pairwise assertions; reduce testNodes to the Phase 5 single document.
'use strict';
var div=document.createElement('div'),child=document.createElement('span');
document.body.appendChild(div);div.appendChild(child);
var testNodes=[document,document.documentElement,document.body,div,child,document.createTextNode('text'),document.createElement('aside')];
testNodes.forEach(function(reference){
  test(function(){assert_false(reference.contains(null));});
  testNodes.forEach(function(other){
    test(function(){
      var ancestor=other;
      while(ancestor&&ancestor!==reference)ancestor=ancestor.parentNode;
      if(ancestor===reference)assert_true(reference.contains(other));
      else assert_false(reference.contains(other));
    });
  });
});
