onmessage = async event => {
    await Promise.resolve();
    let total = 0n;
    for (const item of event.data.values()) total += BigInt(item.completed ? 0 : 1);
    postMessage({ pending: total, dom: typeof document, collection: event.data instanceof Map });
};
