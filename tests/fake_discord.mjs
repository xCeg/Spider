// A simulated Discord renderer behind a debug port, for Spider's CDP test (`tests/cdp.rs`).
//
// It answers `/json` like Chromium and runs every `Runtime.evaluate` it receives in a page
// that holds a fake webpack module cache: a Flux dispatcher, an activity store, another store,
// and an i18n message proxy that answers any property (`dispatch` and `_dispatcher` included),
// the trap the real script has to step over. The script Spider sends to the real Discord is the
// one that runs here. No dependency: plain Node.
//
// Pages: Discord (stable) behind a service worker and the in-game overlay (both must be
// skipped), Discord PTB, and Discord Canary, whose store only shows a card 60 ms after the
// dispatch (the "slow tick" the engine must wait for). `GET /state/<page>` returns the page's
// activities by socket id, for the test to check.

import crypto from "node:crypto";
import http from "node:http";
import vm from "node:vm";

function discord(slow) {
  const activities = new Map();
  const dispatcher = {
    dispatch(action) {
      if (action.type !== "LOCAL_ACTIVITY_UPDATE") return;
      const apply = () => {
        if (action.activity == null) activities.delete(action.socketId);
        else activities.set(action.socketId, JSON.parse(JSON.stringify(action.activity)));
      };
      if (slow) setTimeout(apply, 60);
      else apply();
    },
    subscribe() {},
  };
  const activityStore = { _dispatcher: dispatcher, getActivities: () => [...activities.values()] };
  const otherStore = { _dispatcher: dispatcher, getName: () => "other" };
  const intl = new Proxy(function () {}, {
    get: (target, key) => (key === Symbol.toStringTag ? "IntlMessagesProxy" : () => undefined),
  });
  const cache = {
    10: { exports: intl },
    11: { exports: { Z: otherStore } },
    12: { exports: { default: activityStore } },
    13: { exports: null },
  };
  const sandbox = { setTimeout };
  sandbox.window = sandbox;
  sandbox.webpackChunkdiscord_app = { push: (entry) => entry[2]({ c: cache }) };
  return { context: vm.createContext(sandbox), activities };
}

const pages = {
  worker: { type: "service_worker", url: "https://discord.com/assets/sw.js", page: discord(false) },
  overlay: { type: "page", url: "https://discord.com/overlay?pid=1", page: discord(false) },
  stable: { type: "page", url: "https://discord.com/channels/@me", page: discord(false) },
  ptb: { type: "page", url: "https://ptb.discord.com/channels/@me", page: discord(false) },
  canary: { type: "page", url: "https://canary.discord.com/app", page: discord(true) },
};

function reply(response, value) {
  const body = JSON.stringify(value);
  response.writeHead(200, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(body),
  });
  response.end(body);
}

const server = http.createServer((request, response) => {
  const port = server.address().port;
  if (request.url === "/json" || request.url === "/json/list") {
    reply(
      response,
      Object.entries(pages).map(([id, page]) => ({
        id,
        type: page.type,
        url: page.url,
        webSocketDebuggerUrl: `ws://127.0.0.1:${port}/devtools/page/${id}`,
      })),
    );
  } else if (request.url.startsWith("/state/")) {
    const page = pages[request.url.slice("/state/".length)];
    reply(response, page ? Object.fromEntries(page.page.activities) : null);
  } else {
    response.writeHead(404, { "Content-Length": 0 });
    response.end();
  }
});

function send(socket, text) {
  const data = Buffer.from(text, "utf8");
  let head;
  if (data.length < 126) {
    head = Buffer.from([0x81, data.length]);
  } else if (data.length < 65536) {
    head = Buffer.alloc(4);
    head[0] = 0x81;
    head[1] = 126;
    head.writeUInt16BE(data.length, 2);
  } else {
    head = Buffer.alloc(10);
    head[0] = 0x81;
    head[1] = 127;
    head.writeBigUInt64BE(BigInt(data.length), 2);
  }
  socket.write(Buffer.concat([head, data]));
}

server.on("upgrade", (request, socket) => {
  const target = pages[request.url.split("/").pop()];
  if (!target) {
    socket.destroy();
    return;
  }
  const accept = crypto
    .createHash("sha1")
    .update(request.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  let buffer = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      if (buffer.length < 2) return;
      const opcode = buffer[0] & 0x0f;
      const masked = (buffer[1] & 0x80) !== 0;
      let length = buffer[1] & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (buffer.length < 4) return;
        length = buffer.readUInt16BE(2);
        offset = 4;
      } else if (length === 127) {
        if (buffer.length < 10) return;
        length = Number(buffer.readBigUInt64BE(2));
        offset = 10;
      }
      const maskAt = offset;
      if (masked) offset += 4;
      if (buffer.length < offset + length) return;
      let payload = buffer.subarray(offset, offset + length);
      if (masked) {
        const mask = buffer.subarray(maskAt, maskAt + 4);
        payload = Buffer.from(payload.map((byte, index) => byte ^ mask[index % 4]));
      }
      buffer = buffer.subarray(offset + length);
      if (opcode === 0x8) {
        socket.end();
        return;
      }
      if (opcode !== 0x1) continue;
      const message = JSON.parse(payload.toString("utf8"));
      let answer;
      try {
        const value = vm.runInContext(message.params.expression, target.page.context);
        answer = { id: message.id, result: { result: { type: typeof value, value } } };
      } catch (error) {
        answer = { id: message.id, result: { exceptionDetails: { text: String(error) } } };
      }
      send(socket, JSON.stringify(answer));
    }
  });
});

server.listen(0, "127.0.0.1", () => {
  // The test reads the port from this first line.
  console.log(server.address().port);
});
