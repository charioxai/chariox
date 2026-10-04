"""MP-08 / MP-10, WP-12: privileged official password-substring scorer.

Only allowlisted task metadata leaves manifest mode. Targets never leave this
process or enter a solver prompt. No browser actions or alternate provider runtime.
"""
import json
import sys


def main():
    with open(sys.argv[1], encoding="utf-8") as source:
        tasks = [json.loads(line) for line in source if line.strip()]
    if sys.argv[2] == "manifest":
        fields = ("id", "path", "title", "description", "difficulty", "variant", "base_task")
        result = {"count": len(tasks), "tasks": [
            {key: task[key] for key in fields if key in task} for task in tasks
        ]}
    elif sys.argv[2] == "score":
        request = json.load(sys.stdin)
        task = next(task for task in tasks if task["id"] == request["id"])
        assert isinstance(task["password"], str) and task["password"]
        assert isinstance(request["answer"], str)
        result = {"reward": int(task["password"] in request["answer"]),
                  "scorer": "official target.text in state.output.completion"}
    else:
        raise ValueError("MP-10 unknown evaluator operation")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
