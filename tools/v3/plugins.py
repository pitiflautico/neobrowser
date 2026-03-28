"""
NeoBrowser Plugin Engine — execute YAML pipelines via browser tools.

Plugins are YAML files in ~/.neorender/plugins/ that chain browser actions.
Each step calls a neo-browser tool with template variables.
"""

import json, os, re, time, glob
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

PLUGIN_DIR = Path.home() / '.neorender' / 'plugins'
PLUGIN_DIR.mkdir(parents=True, exist_ok=True)


def resolve_template(text, ctx):
    """Simple {{var}} template resolution."""
    if not isinstance(text, str):
        return text

    def replacer(m):
        key = m.group(1).strip()
        # Support nested: ctx[a][b]
        parts = key.split('.')
        val = ctx
        for p in parts:
            if isinstance(val, dict):
                val = val.get(p, '')
            elif isinstance(val, list) and p.isdigit():
                val = val[int(p)] if int(p) < len(val) else ''
            else:
                val = ''
                break
        return str(val) if val is not None else ''

    return re.sub(r'\{\{(.+?)\}\}', replacer, text)


def resolve_obj(obj, ctx):
    """Recursively resolve templates in dicts/lists/strings."""
    if isinstance(obj, str):
        return resolve_template(obj, ctx)
    elif isinstance(obj, dict):
        return {k: resolve_obj(v, ctx) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [resolve_obj(v, ctx) for v in obj]
    return obj


def load_plugin(name):
    """Load a plugin by name from the plugins directory."""
    if not yaml:
        return None, 'PyYAML not installed. Run: pip install pyyaml'

    # Try exact file
    path = PLUGIN_DIR / f'{name}.yaml'
    if not path.exists():
        path = PLUGIN_DIR / f'{name}.yml'
    if not path.exists():
        # Search by name field inside YAML files
        for f in PLUGIN_DIR.glob('*.y*ml'):
            try:
                data = yaml.safe_load(f.read_text())
                if data and data.get('name') == name:
                    return data, None
            except:
                pass
        return None, f'Plugin not found: {name}'

    try:
        data = yaml.safe_load(path.read_text())
        return data, None
    except Exception as e:
        return None, f'Plugin parse error: {e}'


def list_plugins():
    """List all available plugins with descriptions."""
    plugins = []
    for f in sorted(PLUGIN_DIR.glob('*.y*ml')):
        try:
            if yaml:
                data = yaml.safe_load(f.read_text())
                plugins.append({
                    'name': data.get('name', f.stem),
                    'description': data.get('description', ''),
                    'file': str(f),
                    'inputs': list(data.get('inputs', {}).keys()),
                    'steps': len(data.get('steps', [])),
                })
            else:
                plugins.append({'name': f.stem, 'file': str(f)})
        except:
            plugins.append({'name': f.stem, 'file': str(f), 'error': 'parse failed'})
    return plugins


def create_plugin(name, description, steps_yaml):
    """Create a new plugin file."""
    path = PLUGIN_DIR / f'{name}.yaml'
    if path.exists():
        return f'Plugin {name} already exists at {path}'

    content = steps_yaml if isinstance(steps_yaml, str) else yaml.dump(steps_yaml, default_flow_style=False)
    path.write_text(content)
    return f'Plugin created: {path}'


def run_plugin(plugin_data, user_inputs, tool_dispatch):
    """
    Execute a plugin pipeline.

    Args:
        plugin_data: parsed YAML dict
        user_inputs: dict of input values from the user
        tool_dispatch: function(tool_name, args) → result string
    """
    # Build context with defaults + user inputs
    ctx = {'timestamp': time.strftime('%Y-%m-%d %H:%M:%S')}
    for key, spec in plugin_data.get('inputs', {}).items():
        default = spec.get('default', '') if isinstance(spec, dict) else ''
        ctx[key] = user_inputs.get(key, default)

    results = []
    step_data = {}  # save_as storage

    for i, step in enumerate(plugin_data.get('steps', [])):
        action = step.get('action', '')
        loop_var = step.get('loop')
        loop_as = step.get('as', 'item')
        repeat = int(step.get('repeat', 1))
        save_as = step.get('save_as', '')

        # Build args for this step (resolve templates)
        step_args = {}
        for k, v in step.items():
            if k not in ('action', 'loop', 'as', 'repeat', 'save_as'):
                step_args[k] = resolve_obj(v, {**ctx, **step_data})

        if loop_var and loop_var in ctx:
            items = ctx[loop_var]
            if isinstance(items, str):
                items = [x.strip() for x in items.split(',')]

            for item in items:
                loop_ctx = {**ctx, **step_data, loop_as: item}
                resolved_args = resolve_obj(step_args, loop_ctx)

                for r in range(repeat):
                    result = tool_dispatch(action, resolved_args)
                    results.append(f'[step {i+1}/{loop_as}={item}] {action}: {str(result)[:200]}')

                if save_as:
                    resolved_save = resolve_template(save_as, loop_ctx)
                    step_data[resolved_save] = result
        else:
            resolved_args = resolve_obj(step_args, {**ctx, **step_data})
            for r in range(repeat):
                result = tool_dispatch(action, resolved_args)
                results.append(f'[step {i+1}] {action}: {str(result)[:200]}')

            if save_as:
                resolved_save = resolve_template(save_as, {**ctx, **step_data})
                step_data[resolved_save] = result

    # Format output
    output_spec = plugin_data.get('output', {})
    template = output_spec.get('template', '')
    if template:
        return resolve_template(template, {**ctx, **step_data})

    return '\n'.join(results)
