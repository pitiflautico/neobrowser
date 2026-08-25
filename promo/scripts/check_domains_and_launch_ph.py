import asyncio
import json
import os
import subprocess
import sys

# Ordered by preference. The first PR that is merged AND whose subdomain
# returns HTTP 200 triggers the Product Hunt launch.
CANDIDATES = [
    {
        "name": "is-a-good.dev",
        "pr_api": "https://api.github.com/repos/is-a-good-dev/register/pulls/1295",
        "domain": "https://neobrowser.is-a-good.dev/",
    },
    {
        "name": "is-amaz.ing",
        "pr_api": "https://api.github.com/repos/is-amazing/register/pulls/297",
        "domain": "https://neobrowser.is-amaz.ing/",
    },
    {
        "name": "creepers.sbs",
        "pr_api": "https://api.github.com/repos/creepersbs/register/pulls/133",
        "domain": "https://neobrowser.creepers.sbs/",
    },
]


def pr_merged(pr_api):
    try:
        out = subprocess.check_output(["curl", "-s", pr_api], text=True)
        d = json.loads(out)
        return d.get("state") == "closed" and d.get("merged") is True
    except Exception as e:
        print(f"PR check failed for {pr_api}: {e}")
        return False


def site_ready(domain):
    try:
        out = subprocess.check_output(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", domain],
            text=True,
        )
        return out.strip() == "200"
    except Exception as e:
        print(f"Site check failed for {domain}: {e}")
        return False


async def main():
    ready = None
    for c in CANDIDATES:
        print(f"Checking {c['name']}...")
        if not pr_merged(c["pr_api"]):
            print(f"  {c['name']} PR not merged yet.")
            continue
        if not site_ready(c["domain"]):
            print(f"  {c['name']} PR merged but site not returning 200 yet.")
            continue
        ready = c
        break

    if ready is None:
        print("No domain ready yet. Exiting.")
        sys.exit(0)

    print(f"{ready['name']} ready ({ready['domain']}). Launching Product Hunt...")
    env = {
        **os.environ,
        "NEOBROWSER_PH_WEBSITE": ready["domain"],
    }
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        "promo/scripts/producthunt_launch.py",
        cwd=os.path.expanduser("/Volumes/DiscoExterno2/mac_offload/Projects/neobrowser"),
        env=env,
    )
    await proc.wait()


if __name__ == "__main__":
    asyncio.run(main())
