// Yerel yardimci: index.html'i sunar, tarayicidan gelen GIF baytlarini assets/ altina yazar.
const http = require('http'), fs = require('fs'), path = require('path');
const out = path.join(__dirname, '..', '..', 'assets', 'claude-spin.gif');
http.createServer((req, res) => {
  if (req.method === 'POST' && req.url === '/save') {
    const chunks = [];
    req.on('data', c => chunks.push(c));
    req.on('end', () => { fs.writeFileSync(out, Buffer.concat(chunks)); res.end('ok'); console.log('saved', out); });
  } else {
    res.setHeader('Content-Type', 'text/html; charset=utf-8');
    res.end(fs.readFileSync(path.join(__dirname, 'index.html')));
  }
}).listen(8765, '127.0.0.1', () => console.log('http://127.0.0.1:8765'));
