 var o=new MutationObserver(()=>o.disconnect());o.observe(root,{subtree:true,attributes:true,attributeOldValue:true,childList:true});root.setAttribute('x','1');root.textContent='changed';
