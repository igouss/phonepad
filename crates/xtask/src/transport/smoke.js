const report = (line) => console.log(`WT ${line}`);
let units = 0;
const held = new Map();
let next = 0;
const ackAll = (writer) => {
  while (held.has(next)) {
    const ack = new Uint8Array(4);
    new DataView(ack.buffer).setUint32(0, held.get(next));
    held.delete(next);
    next += 1;
    units += 1;
    writer.write(ack);
  }
};
const run = async () => {
  const transport = new WebTransport(TARGET, {
    serverCertificateHashes: [{ algorithm: "sha-256", value: new Uint8Array(HASH) }],
  });
  await transport.ready;
  report("ready");
  const control = await transport.createBidirectionalStream();
  const acks = control.writable.getWriter();
  await acks.write(new Uint8Array([99]));
  const ping = await transport.createBidirectionalStream();
  await ping.writable.getWriter().write(new Uint8Array([112]));
  const incoming = transport.incomingUnidirectionalStreams.getReader();
  for (;;) {
    const { value, done } = await incoming.read();
    if (done) {
      return;
    }
    new Response(value).arrayBuffer().then((buffer) => {
      const view = new DataView(buffer);
      held.set(view.getUint32(1), view.getUint32(5));
      ackAll(acks);
    });
  }
};
run().catch((error) => report(`error ${error}`));
setInterval(() => report(`units ${units}`), 1000);
