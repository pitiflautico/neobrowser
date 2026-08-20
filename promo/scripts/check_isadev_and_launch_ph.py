import asyncio
import json
import os
import subprocess
import sys

PR_API = "https://api.github.com/repos/is-a-dev/register/pulls/48212"
DOMAIN = "https://neobrowser.is-a.dev/"


def pr_merged():
    try:
        out = subprocess.check_output(["curl", "-s", PR_API], text=True)
        d = json.loads(out)
        return d.get("state") == "closed" and d.get("merged") is True
    except Exception as e:
        print("PR check failed:", e)
        return False


def site_ready():
    try:
        out = subprocess.check_output(
            ["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", DOMAIN],
            text=True,
        )
        return out.strip() == "200"
    except Exception as e:
        print("Site check failed:", e)
        return False


async def main():
    if not pr_merged():
        print("is-a-dev PR not merged yet. Exiting.")
        sys.exit(0)
    if not site_ready():
        print("PR merged but site not returning 200 yet. Exiting.")
        sys.exit(0)
    print("PR merged and site ready. Launching Product Hunt...")
    # run PH launch v3
    proc = await asyncio.create_subprocess_exec(
        sys.executable,
        "promo/scripts/producthunt_launch_v3.py",
        cwd=os.path.expanduser("/Volumes/DiscoExterno2/mac_offload/Projects/neobrowser"),
    )
    await proc.wait()


if __name__ == "__main__":
    asyncio.run(main())
