const http = require("node:http");

http.createServer((request, response) => {
  response.writeHead(200, { "Content-Type": "text/plain" });
  response.end("Repo Doctor startup fixture\n");
}).listen(3001, "127.0.0.1");
