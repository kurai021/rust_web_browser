'use strict';
let expanded=false;
document.getElementById('change').addEventListener('click',()=>{
  expanded=!expanded;
  document.getElementById('mutation').textContent=expanded
    ? 'This longer paragraph changes one card. The grid recalculates the affected row and reuses compatible measurements and geometry elsewhere. Click again to restore the short version.'
    : 'Only this card changes; other subtrees can be reused.';
});
