globalThis.detailsLoads = (globalThis.detailsLoads || 0) + 1;
export function describe() { return 'Loaded with import() · cached namespace · live module bindings'; }
export const source = import.meta.url;
