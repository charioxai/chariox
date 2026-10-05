"""MP-08 / MP-10: retain exact official prompt text without importing its SDK."""
import ast, json, pathlib, sys
source, task, answer, count, output = sys.argv[1:]
values = {}
for node in ast.parse(pathlib.Path(source).read_text()).body:
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id in ('SYSTEM_PROMPT', 'USER_PROMPT'):
                values[target.id] = ast.literal_eval(node.value)
assert set(values) == {'SYSTEM_PROMPT', 'USER_PROMPT'}
user = values['USER_PROMPT'].replace('<task>', task).replace('<answer>', pathlib.Path(answer).read_text()).replace('<num>', count)
pathlib.Path(output).write_text(json.dumps({'system': values['SYSTEM_PROMPT'], 'user': user + '\nYour verdict:\n'}))
