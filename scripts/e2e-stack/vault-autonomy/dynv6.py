"""MP-08 / MP-09 / MP-10 / MP-11: real dynv6 drill client.

Only capture writes a credential to stdout; invoke it through Vault capture_command.
--state must live in the lane's private state directory, outside the fixture.
"""

import argparse
import http.cookiejar
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from html.parser import HTMLParser

BASE = "https://dynv6.com"


class Page(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.forms = []
        self.links = []
        self.inputs = []
        self.key_values = []
        self.account_url = None
        self.email_confirmed = None
        self.text = []
        self.hidden_depth = 0
        self.form = None
        self.csrf = None
        self.feed(text)

    def handle_starttag(self, tag, items):
        a = dict(items)
        if tag in ["script", "style"]:
            self.hidden_depth += 1
        if tag == "form":
            self.form = {
                "action": a.get("action", ""),
                "method": a.get("method", "get").lower(),
                "fields": {},
            }
            self.forms.append(self.form)
        if tag == "input":
            self.inputs.append(a)
            if (
                self.form
                and a.get("name")
                and a.get("type") not in ["submit", "button"]
                and (a.get("type") not in ["checkbox", "radio"] or "checked" in a)
            ):
                self.form["fields"][a["name"]] = a.get("value", "")
        if tag in ["a", "b-nav-item", "b-dropdown-item"]:
            self.links.append(a)
        if tag == "key-data" and ":k" in a:
            self.key_values.append(json.loads(a[":k"])["data"])
        if tag == "credential-settings":
            self.account_url = a.get("account-url")
        if tag == "email-form" and "attributes" in a:
            self.email_confirmed = bool(json.loads(a["attributes"]).get("confirmed_at"))
        if tag == "meta" and a.get("name") == "csrf-token":
            self.csrf = a.get("content")

    def handle_endtag(self, tag):
        if tag in ["script", "style"]:
            self.hidden_depth = max(0, self.hidden_depth - 1)
        if tag == "form":
            self.form = None

    def handle_data(self, data):
        if not self.hidden_depth:
            self.text.append(data)


def fetch(opener, path, fields=None, method=None, csrf=None):
    url = urllib.parse.urljoin(BASE, path)
    if urllib.parse.urlparse(url).netloc != "dynv6.com":
        raise RuntimeError("unexpected service origin")
    req = urllib.request.Request(
        url,
        data=urllib.parse.urlencode(fields).encode() if fields is not None else None,
        method=method,
        headers={"X-CSRF-Token": csrf} if csrf else {},
    )
    return opener.open(req, timeout=25)


def login():
    auth = json.loads(STATE.read_text())
    opener = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
    )
    p = Page(fetch(opener, "/users/sign_in").read().decode())
    form = next(f for f in p.forms if "user[email]" in f["fields"])
    fields = form["fields"]
    fields["user[email]"] = auth["email"]
    fields["user[password]"] = auth["password"]
    r = fetch(opener, form["action"], fields)
    body = r.read().decode()
    if not authenticated_page(Page(body)):
        raise RuntimeError("service login did not complete")
    return opener


def api(token, path, body=None, method=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        BASE + "/api/v2" + path,
        data=data,
        method=method,
        headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=25) as r:
        return json.load(r) if r.status != 204 else None


def authenticated_page(page):
    return any(
        urllib.parse.urlparse(a.get("href", "")).path
        in ["/users/sign_out", "/users/edit"]
        for a in page.links
    )


def signup_result(status, url, body):
    path = urllib.parse.urlparse(url).path
    page = Page(body)
    text = " ".join(page.text)
    authenticated = (
        200 <= status < 300
        and path in ["/zones", "/zones/new"]
        and authenticated_page(page)
    )
    confirmation = 200 <= status < 300 and bool(
        re.search(
            r"(confirmation link has been sent|sent.{0,80}confirmation|confirmation.{0,80}sent|confirm your email)",
            text,
            re.I,
        )
    )
    return {
        "registered": authenticated or confirmation,
        "authenticated": authenticated,
        "confirmation_required": confirmation,
        "status": status,
        "path": path,
        "known_error_flags": [
            x
            for x in [
                "Email has already been taken",
                "Email is invalid",
                "not allowed",
                "prohibited",
                "captcha",
            ]
            if x.lower() in text.lower()
        ],
    }


def save_setup(auth):
    if not STATE.is_absolute():
        raise RuntimeError("state path must be absolute and outside the fixture")
    if STATE.resolve().is_relative_to(Path.cwd().resolve()):
        raise RuntimeError("state must be outside the disposable workspace")
    STATE.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # Retain cleanup access even if the service commits signup and the response is lost.
    fd = os.open(STATE, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as out:
        json.dump({"email": auth["email"], "password": auth["password"]}, out)
        out.flush()
        os.fsync(out.fileno())


def signup(auth):
    save_setup(auth)
    op = urllib.request.build_opener(
        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
    )
    p = Page(fetch(op, "/users/sign_up").read().decode())
    form = next(f for f in p.forms if "user[password_confirmation]" in f["fields"])
    fields = form["fields"]
    fields["user[email]"] = auth["email"]
    fields["user[password]"] = auth["password"]
    fields["user[password_confirmation]"] = auth["password"]
    fields["commit"] = "Sign up"
    try:
        r = fetch(op, form["action"], fields)
        body = r.read().decode()
        status = r.status
        url = r.geturl()
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        status = e.code
        url = e.geturl()
    result = signup_result(status, url, body)
    if result["authenticated"]:
        login()
        result["fresh_login_verified"] = True
    return result


def delete_account():
    auth = json.loads(STATE.read_text())
    acknowledged = auth.get("deletion_acknowledged", False)
    if not acknowledged:
        op = login()
        page = Page(fetch(op, "/users/edit").read().decode())
        if not page.account_url:
            raise RuntimeError(
                "account cancellation control not found; private setup retained"
            )
        response = fetch(
            op,
            page.account_url,
            {"current_password": auth["password"]},
            method="DELETE",
            csrf=page.csrf,
        )
        result = json.loads(response.read().decode())
        location = result["location"]
        if not 200 <= response.status < 300 or urllib.parse.urlparse(
            location
        ).path not in ["/", "/users/sign_in", "/users/sign_up"]:
            raise RuntimeError(
                "service did not acknowledge account cancellation; private setup retained"
            )
        # Preserve the service's successful cancellation acknowledgement before
        # navigation or login verification can fail. A later cleanup can reconcile it.
        auth["deletion_acknowledged"] = True
        with STATE.open("w") as out:
            json.dump(auth, out)
            out.flush()
            os.fsync(out.fileno())
        fetch(op, location).read()
    try:
        login()
    except RuntimeError as error:
        if str(error) != "service login did not complete":
            raise
    else:
        raise RuntimeError(
            "account still accepts login after cancellation; private setup retained"
        )
    STATE.unlink()
    print(
        json.dumps(
            {
                "account_deleted": True,
                "service_cancellation_acknowledged": True,
                "fresh_login_refused": True,
                "private_setup_removed": True,
            }
        )
    )


def main():
    global STATE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True, type=Path)
    parser.add_argument(
        "mode",
        choices=[
            "signup",
            "status",
            "capture",
            "red",
            "create",
            "probe",
            "delete-zone",
            "delete-account",
        ],
    )
    parser.add_argument("args", nargs="*")
    args = parser.parse_args()
    STATE = args.state
    mode = args.mode
    if mode == "signup":
        print(json.dumps(signup(json.load(sys.stdin))))
    elif mode == "status":
        op = login()
        page = Page(fetch(op, "/users/edit").read().decode())
        if page.email_confirmed is None:
            raise RuntimeError("service did not report account confirmation state")
        print(
            json.dumps({"authenticated": True, "email_confirmed": page.email_confirmed})
        )
    elif mode == "capture":
        op = login()
        page = Page(fetch(op, "/keys").read().decode())
        Path("dynv6-keys-structure.json").write_text(
            json.dumps({"key_components": len(page.key_values)})
        )
        for token in page.key_values[:8]:
            try:
                valid = isinstance(api(token, "/zones"), list)
            except urllib.error.HTTPError as error:
                if error.code not in [401, 403]:
                    raise
                continue
            if valid:
                print(token)
                break
        else:
            raise RuntimeError(
                "no verified token in the service's key-data components; private setup retained"
            )
    elif mode == "red":
        try:
            urllib.request.urlopen(BASE + "/api/v2/zones", timeout=25)
            raise RuntimeError("unauthenticated API unexpectedly accepted")
        except urllib.error.HTTPError as e:
            assert e.code in [401, 403]
            print(json.dumps({"red": True, "status": e.code}))
    elif mode == "create":
        token = os.environ["DYNV6_TOKEN"]
        name = args.args[0]
        proof = args.args[1]
        zone = api(token, "/zones", {"name": name})
        state = {
            "zone_id": zone["id"],
            "fqdn": "_vaultproof." + name,
            "proof": proof,
        }
        Path("dynv6-state.json").write_text(json.dumps(state))
        record = api(
            token,
            "/zones/" + str(zone["id"]) + "/records",
            {"type": "TXT", "name": "_vaultproof", "data": proof},
        )
        state["record_id"] = record["id"]
        Path("dynv6-state.json").write_text(json.dumps(state))
        print(json.dumps({"created": True, "zone": name, "fqdn": state["fqdn"]}))
    elif mode == "probe":
        state = json.load(open("dynv6-state.json"))
        req = urllib.request.Request(
            "https://cloudflare-dns.com/dns-query?"
            + urllib.parse.urlencode({"name": state["fqdn"], "type": "TXT"}),
            headers={"Accept": "application/dns-json"},
        )
        with urllib.request.urlopen(req, timeout=25) as r:
            x = json.load(r)
        answers = [v.get("data", "").strip('"') for v in x.get("Answer", [])]
        out = {
            "resolved": state["proof"] in answers,
            "fqdn": state["fqdn"],
            "answers": answers,
            "status": x.get("Status"),
        }
        json.dump(out, open("dynv6-proof.json", "w"))
        print(json.dumps(out))
        assert out["resolved"], "public TXT not resolved yet"
    elif mode == "delete-zone":
        state = json.load(open("dynv6-state.json"))
        api(
            os.environ["DYNV6_TOKEN"],
            "/zones/" + str(state["zone_id"]),
            method="DELETE",
        )
        print(json.dumps({"deleted": True}))
    elif mode == "delete-account":
        delete_account()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # HTTP error bodies/URLs may contain credentials. Keep failures bounded and safe.
        reason = str(error) if type(error) is RuntimeError else "dynv6 request failed"
        print(
            json.dumps(
                {
                    "error": type(error).__name__,
                    "http_status": (
                        error.code
                        if isinstance(error, urllib.error.HTTPError)
                        else None
                    ),
                    "message": reason,
                    "next_action": "retain private setup and retry cleanup",
                }
            ),
            file=sys.stderr,
        )
        sys.exit(1)
