// Adapted from WPT fetch/api/basic/request-headers.any.js (BSD-3-Clause).
function checkContentType(contentType,body){
  if(body instanceof FormData){assert_true(contentType.startsWith('multipart/form-data; boundary='));return;}
  var expected='text/plain;charset=UTF-8';if(body===null)expected=null;else if(body instanceof Blob)expected=body.type?body.type:null;else if(body instanceof URLSearchParams)expected='application/x-www-form-urlencoded;charset=UTF-8';assert_equals(contentType,expected);
}
function requestHeaders(desc,method,body,expectedOrigin,expectedLength){promise_test(function(t){if(typeof body==='function')body=body();var options={method:method};if(body)options.body=body;return fetch('/inspect',options).then(t.step_func(function(resp){assert_equals(resp.status,200);assert_equals(resp.type,'basic');assert_true(resp.headers.has('x-request-user-agent'));assert_false(resp.headers.has('accept-charset'));assert_equals(resp.headers.get('x-request-origin'),expectedOrigin);if(expectedLength!==undefined)assert_equals(resp.headers.get('x-request-content-length'),expectedLength);checkContentType(resp.headers.get('x-request-content-type'),body);}));},desc);}
requestHeaders('Fetch with GET','GET',null,null,null);
requestHeaders('Fetch with HEAD','HEAD',null,null,null);
requestHeaders('Fetch with PUT without body','POST',null,location.origin,'0');
requestHeaders('Fetch with PUT with body','PUT',"Request's body",location.origin,'14');
requestHeaders('Fetch with POST without body','POST',null,location.origin,'0');
requestHeaders('Fetch with POST with text body','POST',"Request's body",location.origin,'14');
requestHeaders('Fetch with POST with FormData body','POST',function(){return new FormData();},location.origin);
requestHeaders('Fetch with POST with URLSearchParams body','POST',function(){return new URLSearchParams('name=value');},location.origin,'10');
requestHeaders('Fetch with POST with Blob body','POST',new Blob(['Test']),location.origin,'4');
requestHeaders('Fetch with POST with Blob body with mime type','POST',new Blob(['Test'],{type:'text/maybe'}),location.origin,'4');
requestHeaders('Fetch with Chicken','Chicken',null,location.origin,null);
requestHeaders('Fetch with Chicken with body','Chicken',"Request's body",location.origin,'14');
function requestOriginHeader(method,mode,needsOrigin){promise_test(function(t){return fetch('/inspect',{method:method,mode:mode}).then(t.step_func(function(resp){assert_equals(resp.status,200);assert_equals(resp.type,'basic');assert_equals(resp.headers.get('x-request-origin'),needsOrigin?location.origin:null);}));},'Fetch with '+method+' and mode "'+mode+'" '+(needsOrigin?'needs':'does not need')+' an Origin header');}
requestOriginHeader('GET','cors',false);requestOriginHeader('POST','same-origin',true);requestOriginHeader('POST','no-cors',true);requestOriginHeader('PUT','same-origin',true);requestOriginHeader('TacO','same-origin',true);requestOriginHeader('TacO','cors',true);
['ArrayBuffer','Uint8Array','Int8Array','Float16Array','Float32Array','Float64Array','DataView'].forEach(function(type){skip('Fetch with POST with '+type+' body','Typed binary arrays are not part of the current Level 1 language surface.');});
