"""TEST ONLY: the two-condition run's fixture site, on http://localhost:PORT
inside the computer (a login's site may be http on localhost; no public HTTPS
fixture exists yet). It signs a person in with a password and a TOTP code, or
with a passkey, then takes a post and serves a file to download.

It holds no secret: the password as a salted SHA-256, the TOTP codes for
the run's time window as SHA-256 (never the seed), and the passkey's public
key. It writes nothing and logs nothing; what it receives stays in memory.
GET /_events lists what happened, with no values in it.

usage: python3 site.py CONFIG.json"""
import hashlib
import http.server
import json
import secrets
import sys
import time
import urllib.parse
from base64 import b64decode, urlsafe_b64decode, urlsafe_b64encode

CONF = json.load(open(sys.argv[1]))
ORIGIN = f"http://localhost:{CONF['port']}"
SESSIONS, PENDING, CHALLENGES, POSTS, EVENTS = {}, set(), set(), [], []

# --- P-256 ECDSA verification, for the passkey's assertion -----------------
P = 2**256 - 2**224 + 2**192 + 2**96 - 1
N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
G = (0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296,
     0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5)


def _add(p1, p2):
    if p1 is None:
        return p2
    if p2 is None:
        return p1
    if p1[0] == p2[0] and (p1[1] + p2[1]) % P == 0:
        return None
    if p1 == p2:
        m = (3 * p1[0] * p1[0] - 3) * pow(2 * p1[1], -1, P) % P
    else:
        m = (p2[1] - p1[1]) * pow(p2[0] - p1[0], -1, P) % P
    x = (m * m - p1[0] - p2[0]) % P
    return x, (m * (p1[0] - x) - p1[1]) % P


def _mul(k, point):
    out = None
    while k:
        if k & 1:
            out = _add(out, point)
        point = _add(point, point)
        k >>= 1
    return out


def _der(sig):
    assert sig[0] == 0x30
    i = 2
    assert sig[i] == 2
    r = int.from_bytes(sig[i + 2:i + 2 + sig[i + 1]], "big")
    i += 2 + sig[i + 1]
    assert sig[i] == 2
    s = int.from_bytes(sig[i + 2:i + 2 + sig[i + 1]], "big")
    return r, s


def verify(pub, message, sig):
    r, s = _der(sig)
    if not (0 < r < N and 0 < s < N):
        return False
    e = int.from_bytes(hashlib.sha256(message).digest(), "big")
    w = pow(s, -1, N)
    point = _add(_mul(e * w % N, G), _mul(r * w % N, pub))
    return point is not None and point[0] % N == r


PUB = (int(CONF["pub_x"], 16), int(CONF["pub_y"], 16))


def b64u(data):
    return urlsafe_b64decode(data + "=" * (-len(data) % 4))


# --- pages ---------------------------------------------------------------------
def page(title, body):
    return (f"<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head>"
            f"<body><h1>{title}</h1>{body}</body></html>")


PASSKEY_JS = """
<script>
const b = s => Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')), c => c.charCodeAt(0));
const u = a => btoa(String.fromCharCode(...new Uint8Array(a))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,'');
async function go() {
  const out = document.getElementById('out');
  try {
    const a = await navigator.credentials.get({publicKey: {challenge: b('%s'), rpId: 'localhost',
      allowCredentials: [{type: 'public-key', id: b('%s')}], userVerification: 'preferred', timeout: 30000}});
    const r = await fetch('/passkey/verify', {method: 'POST', headers: {'content-type': 'application/json'},
      body: JSON.stringify({id: a.id, authenticatorData: u(a.response.authenticatorData),
        clientDataJSON: u(a.response.clientDataJSON), signature: u(a.response.signature)})});
    if (r.ok) { location.href = '/'; } else { out.textContent = 'Passkey refused: ' + await r.text(); }
  } catch (e) { out.textContent = 'Passkey failed: ' + e.name; }
}
</script>
<button onclick="go()">Sign in with a passkey</button><p id=out></p>
"""


class Site(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def who(self):
        cookies = dict(c.strip().split("=", 1) for c in (self.headers.get("Cookie") or "").split(";") if "=" in c)
        return SESSIONS.get(cookies.get("sid")), cookies

    def send(self, status, body="", headers=None, kind="text/html; charset=utf-8"):
        data = body.encode() if isinstance(body, str) else body
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def go(self, where, cookie=None):
        self.send(303, "", {"Location": where, **({"Set-Cookie": cookie} if cookie else {})})

    def form(self):
        n = int(self.headers.get("Content-Length") or 0)
        return {k: v[0] for k, v in urllib.parse.parse_qs(self.rfile.read(n).decode()).items()}

    def signed_in(self, how):
        sid = secrets.token_hex(16)
        SESSIONS[sid] = {"user": CONF["username"], "how": how}
        EVENTS.append({"at": time.time(), "event": f"signed in by {how}"})
        return f"sid={sid}; Path=/; HttpOnly; SameSite=Lax"

    def do_GET(self):
        me, cookies = self.who()
        path = urllib.parse.urlparse(self.path).path
        if path == "/":
            if me:
                return self.send(200, page("Home", f"<p>Signed in as {me['user']} ({me['how']})</p>"
                                                   "<p><a href=/post>Write a post</a></p><p><a href=/files>Files</a></p>"
                                                   "<p><a href=/logout>Sign out</a></p>"))
            return self.send(200, page("Welcome", "<p>Signed out</p><p><a href=/login>Sign in</a></p>"
                                                  "<p><a href=/passkey>Sign in with a passkey</a></p>"))
        if path == "/login":
            return self.send(200, page("Sign in", "<form method=post action=/login>"
                                                  "<label>Username <input name=username type=text autocomplete=username></label>"
                                                  "<label>Password <input name=password type=password autocomplete=current-password></label>"
                                                  "<button>Sign in</button></form>"))
        if path == "/totp":
            return self.send(200, page("Two-step verification", "<form method=post action=/totp>"
                                                                "<label>Code <input name=code type=text inputmode=numeric autocomplete=one-time-code></label>"
                                                                "<button>Verify</button></form>"))
        if path == "/passkey":
            challenge = secrets.token_bytes(32)
            CHALLENGES.add(challenge)
            return self.send(200, page("Passkey sign-in", PASSKEY_JS % (urlsafe_b64encode(challenge).decode().rstrip("="), CONF["cred_id"])))
        if path == "/logout":
            SESSIONS.pop(cookies.get("sid"), None)
            EVENTS.append({"at": time.time(), "event": "signed out"})
            return self.go("/", "sid=; Path=/; Max-Age=0")
        if path == "/_events":
            return self.send(200, json.dumps({"events": EVENTS, "posts": len(POSTS)}), kind="application/json")
        if not me:
            return self.go("/login")
        if path == "/post":
            return self.send(200, page("New post", "<form method=post action=/post>"
                                                   "<label>Post <input name=body type=text></label><button>Publish</button></form>"))
        if path == "/posts":
            return self.send(200, page("Posts", "".join(f"<p>Post {i + 1}: {p}</p>" for i, p in enumerate(POSTS))))
        if path == "/files":
            return self.send(200, page("Files", "<p><a href=/files/report.csv>report.csv</a></p>"))
        if path == "/files/report.csv":
            EVENTS.append({"at": time.time(), "event": "downloaded report.csv"})
            return self.send(200, CONF["report"], {"Content-Disposition": 'attachment; filename="report.csv"'}, kind="text/csv")
        self.send(404, page("Not found", ""))

    def do_POST(self):
        me, _ = self.who()
        path = urllib.parse.urlparse(self.path).path
        if path == "/login":
            f = self.form()
            good = (f.get("username") == CONF["username"] and
                    hashlib.sha256((CONF["pw_salt"] + f.get("password", "")).encode()).hexdigest() == CONF["pw_hash"])
            EVENTS.append({"at": time.time(), "event": "password " + ("accepted" if good else "refused")})
            if not good:
                return self.send(200, page("Sign in", "<p>Wrong username or password</p><a href=/login>Try again</a>"))
            token = secrets.token_hex(16)
            PENDING.add(token)
            return self.go("/totp", f"pre={token}; Path=/; HttpOnly; SameSite=Lax")
        if path == "/totp":
            _, cookies = self.who()
            f = self.form()
            good = cookies.get("pre") in PENDING and hashlib.sha256(f.get("code", "").encode()).hexdigest() in CONF["totp_hashes"]
            EVENTS.append({"at": time.time(), "event": "code " + ("accepted" if good else "refused")})
            if not good:
                return self.send(200, page("Two-step verification", "<p>Wrong code</p><a href=/totp>Try again</a>"))
            PENDING.discard(cookies.get("pre"))
            return self.go("/", self.signed_in("password and code"))
        if path == "/passkey/verify":
            n = int(self.headers.get("Content-Length") or 0)
            a = json.loads(self.rfile.read(n))
            client, auth, sig = b64u(a["clientDataJSON"]), b64u(a["authenticatorData"]), b64u(a["signature"])
            cd = json.loads(client)
            challenge = b64u(cd.get("challenge", ""))
            checks = {
                "type": cd.get("type") == "webauthn.get",
                "origin": cd.get("origin") == ORIGIN,
                "challenge": challenge in CHALLENGES,
                "rpIdHash": auth[:32] == hashlib.sha256(b"localhost").digest(),
                "userPresent": bool(auth[32] & 1),
                "credential": b64u(a["id"]) == b64decode(CONF["cred_id"]),
                "signature": verify(PUB, auth + hashlib.sha256(client).digest(), sig),
            }
            CHALLENGES.discard(challenge)
            failed = [k for k, v in checks.items() if not v]
            EVENTS.append({"at": time.time(), "event": "passkey " + ("verified" if not failed else "refused: " + ",".join(failed))})
            if failed:
                return self.send(400, "refused: " + ",".join(failed), kind="text/plain")
            return self.send(200, "ok", {"Set-Cookie": self.signed_in("passkey")}, kind="text/plain")
        if not me:
            return self.go("/login")
        if path == "/post":
            POSTS.append(self.form().get("body", "")[:200].replace("<", "&lt;"))
            EVENTS.append({"at": time.time(), "event": "posted"})
            return self.go("/posts")
        self.send(404, page("Not found", ""))


http.server.ThreadingHTTPServer(("127.0.0.1", CONF["port"]), Site).serve_forever()
