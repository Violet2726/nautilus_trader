from __future__ import annotations

import atexit
import contextlib
import socket
import ssl
import threading


_tls_tunnel_lock = threading.Lock()
_tls_tunnel_instance: _TLSTunnel | None = None


class _TLSTunnel:
    def __init__(self, local_host: str, local_port: int, remote_host: str, remote_port: int):
        self.local_host = local_host
        self.local_port = local_port
        self.remote_host = remote_host
        self.remote_port = remote_port
        self._stop_event = threading.Event()
        self._server_sock: socket.socket | None = None
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        if self._thread is not None:
            return

        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind((self.local_host, self.local_port))
        server.listen(10)
        server.settimeout(1.0)
        self._server_sock = server

        t = threading.Thread(target=self._serve_forever, name="fix-tls-tunnel", daemon=True)
        self._thread = t
        t.start()

    def stop(self) -> None:
        self._stop_event.set()
        if self._server_sock is not None:
            with contextlib.suppress(Exception):
                self._server_sock.close()

    def _serve_forever(self) -> None:
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE

        while not self._stop_event.is_set():
            try:
                client, _ = self._server_sock.accept() if self._server_sock else (None, None)
                if client is None:
                    break
            except TimeoutError:
                continue
            except OSError:
                break

            client.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

            try:
                upstream = socket.create_connection((self.remote_host, self.remote_port), timeout=10)
                with contextlib.suppress(Exception):
                    upstream.settimeout(None)
                upstream_tls = ctx.wrap_socket(upstream, server_hostname="wind.com.cn")
                with contextlib.suppress(Exception):
                    upstream_tls.settimeout(None)
                upstream_tls.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            except Exception:
                with contextlib.suppress(Exception):
                    client.close()
                continue

            threading.Thread(target=self._pipe, args=(client, upstream_tls), daemon=True).start()
            threading.Thread(target=self._pipe, args=(upstream_tls, client), daemon=True).start()

    @staticmethod
    def _pipe(src: socket.socket, dst: socket.socket) -> None:
        try:
            with contextlib.suppress(Exception):
                while True:
                    data = src.recv(4096)
                    if not data:
                        break
                    dst.sendall(data)
        finally:
            with contextlib.suppress(Exception):
                src.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(Exception):
                dst.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(Exception):
                src.close()
            with contextlib.suppress(Exception):
                dst.close()


def ensure_tls_tunnel(
    remote_host: str,
    remote_port: int,
    local_host: str = "127.0.0.1",
    local_port: int = 16670,
) -> tuple[str, int]:
    global _tls_tunnel_instance
    with _tls_tunnel_lock:
        if _tls_tunnel_instance is None:
            _tls_tunnel_instance = _TLSTunnel(
                local_host=local_host,
                local_port=local_port,
                remote_host=remote_host,
                remote_port=remote_port,
            )
            _tls_tunnel_instance.start()
            atexit.register(_tls_tunnel_instance.stop)
    return local_host, local_port
