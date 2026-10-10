// Copyright web-platform-tests contributors. BSD-3-Clause; see LICENSE.md.
// Adapted from dom/events/AddEventListenerOptions-{once,passive}.any.js,
// Event-dispatch-order-at-target.html and Event-stopImmediatePropagation.html.
'use strict';
test(function() {
  var invoked_once=false, invoked_normal=false;
  function handler_once(){invoked_once=true;}
  function handler_normal(){invoked_normal=true;}
  const et=new EventTarget();
  et.addEventListener('test',handler_once,{once:true});
  et.addEventListener('test',handler_normal);
  et.dispatchEvent(new Event('test'));
  assert_equals(invoked_once,true); assert_equals(invoked_normal,true);
  invoked_once=false; invoked_normal=false;
  et.dispatchEvent(new Event('test'));
  assert_equals(invoked_once,false); assert_equals(invoked_normal,true);
  et.removeEventListener('test',handler_normal);
});
test(function() {
  const et=new EventTarget(); var invoked_count=0;
  function handler(){invoked_count++;if(invoked_count===1)et.dispatchEvent(new Event('test'));}
  et.addEventListener('test',handler,{once:true}); et.dispatchEvent(new Event('test'));
  assert_equals(invoked_count,1);
  invoked_count=0;
  function handler2(){invoked_count++;if(invoked_count===1)et.addEventListener('test',handler2,{once:true});if(invoked_count<=2)et.dispatchEvent(new Event('test'));}
  et.addEventListener('test',handler2,{once:true}); et.dispatchEvent(new Event('test'));
  assert_equals(invoked_count,2);
});
test(function() {
  var invoked_count=0; function handler(){invoked_count++;}
  const et=new EventTarget();
  et.addEventListener('test',handler,{once:true}); et.addEventListener('test',handler);
  et.dispatchEvent(new Event('test')); assert_equals(invoked_count,1);
  invoked_count=0; et.dispatchEvent(new Event('test')); assert_equals(invoked_count,0);
  et.addEventListener('test',handler,{once:true}); et.removeEventListener('test',handler);
  et.dispatchEvent(new Event('test')); assert_equals(invoked_count,0);
});
test(function() {
  const et=new EventTarget(); var invoked_count=0;
  for(let n=4;n>0;n--)et.addEventListener('test',e=>{invoked_count++;e.stopImmediatePropagation();},{once:true});
  for(let n=4;n>0;n--)et.dispatchEvent(new Event('test'));
  assert_equals(invoked_count,4);
});
test(function() {
  var supportsPassive=false;
  var query_options={get passive(){supportsPassive=true;return false;},get dummy(){assert_unreached('dummy getter');return false;}};
  const et=new EventTarget(); et.addEventListener('test_event',null,query_options);
  assert_true(supportsPassive); supportsPassive=false;
  et.removeEventListener('test_event',null,query_options); assert_false(supportsPassive);
});
function testPassiveValue(optionsValue,expectedDefaultPrevented,existingEventTarget,returnValue){
  var defaultPrevented=undefined;
  var handler=function(e){assert_false(e.defaultPrevented);if(returnValue)e.returnValue=false;else e.preventDefault();defaultPrevented=e.defaultPrevented;};
  const et=existingEventTarget||new EventTarget();
  et.addEventListener('test',handler,optionsValue);
  var uncanceled=et.dispatchEvent(new Event('test',{bubbles:true,cancelable:true}));
  assert_equals(defaultPrevented,expectedDefaultPrevented); assert_equals(uncanceled,!expectedDefaultPrevented);
  et.removeEventListener('test',handler,optionsValue);
}
test(function(){testPassiveValue(undefined,true);testPassiveValue({},true);testPassiveValue({passive:false},true);testPassiveValue({passive:true},false);testPassiveValue({passive:0},true);testPassiveValue({passive:1},false);});
test(function(){testPassiveValue(undefined,true,null,true);testPassiveValue({},true,null,true);testPassiveValue({passive:false},true,null,true);testPassiveValue({passive:true},false,null,true);testPassiveValue({passive:0},true,null,true);testPassiveValue({passive:1},false,null,true);});
function testPassiveWithOtherHandlers(optionsValue,expectedDefaultPrevented){
  var handlerInvoked1=false,handlerInvoked2=false;
  function dummyHandler1(){handlerInvoked1=true;} function dummyHandler2(){handlerInvoked2=true;}
  const et=new EventTarget(); et.addEventListener('test',dummyHandler1,{passive:true});et.addEventListener('test',dummyHandler2);
  testPassiveValue(optionsValue,expectedDefaultPrevented,et);
  assert_true(handlerInvoked1);assert_true(handlerInvoked2);
  et.removeEventListener('test',dummyHandler1);et.removeEventListener('test',dummyHandler2);
}
test(function(){testPassiveWithOtherHandlers({},true);testPassiveWithOtherHandlers({passive:false},true);testPassiveWithOtherHandlers({passive:true},false);});
function testOptionEquivalence(a,b,equal){
  var invocationCount=0;function handler(){invocationCount++;}
  const et=new EventTarget();et.addEventListener('test',handler,a);et.addEventListener('test',handler,b);et.dispatchEvent(new Event('test',{bubbles:true}));
  assert_equals(invocationCount,equal?1:2);et.removeEventListener('test',handler,a);et.removeEventListener('test',handler,b);
}
test(function(){testOptionEquivalence({capture:true},{capture:false,passive:false},false);testOptionEquivalence({capture:true},{passive:true},false);testOptionEquivalence({},{passive:false},true);testOptionEquivalence({passive:true},{passive:false},true);testOptionEquivalence(undefined,{passive:true},true);testOptionEquivalence({capture:true,passive:false},{capture:true,passive:true},true);});
test(function(){
  const el=document.createElement('div');const expectedOrder=['capturing','bubbling'];let actualOrder=[];
  el.addEventListener('click',e=>{assert_equals(e.eventPhase,Event.AT_TARGET);actualOrder.push('bubbling');},false);
  el.addEventListener('click',e=>{assert_equals(e.eventPhase,Event.AT_TARGET);actualOrder.push('capturing');},true);
  el.dispatchEvent(new Event('click',{bubbles:true}));assert_array_equals(actualOrder,expectedOrder);
  actualOrder=[];el.dispatchEvent(new Event('click',{bubbles:false}));assert_array_equals(actualOrder,expectedOrder);
});
test(function(){
  const target=document.querySelector('#target');let timesCalled=0;
  target.addEventListener('test',e=>{++timesCalled;e.stopImmediatePropagation();assert_equals(e.cancelBubble,true);});
  target.addEventListener('test',()=>{++timesCalled;});target.dispatchEvent(new Event('test'));
  assert_equals(timesCalled,1);
});
