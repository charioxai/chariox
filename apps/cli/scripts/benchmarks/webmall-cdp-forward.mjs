// MP-08 / MP-10: grader-only TCP forwarding inside the exact owned slice.
import net from 'node:net';
const server = net.createServer(socket => {
  const upstream = net.connect(9222, '127.0.0.1');
  socket.pipe(upstream).pipe(socket);
  socket.on('error', () => upstream.destroy());
  upstream.on('error', () => socket.destroy());
  socket.on('close', () => upstream.destroy());
  upstream.on('close', () => socket.destroy());
});
server.listen(19222, '0.0.0.0');
