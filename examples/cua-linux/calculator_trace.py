"""Read calculation clicks from fresh public Roder desktop observations."""
import json


def calculation_clicks(trace):
    windows = {}
    labels = []
    for call in trace:
        if call.get('status') != 'completed' or call.get('error'):
            continue
        args = call.get('arguments') or {}
        target = (args.get('pid'), args.get('window_id'))
        previous = windows.get(target, {})
        if call.get('tool') == 'cua_click':
            token = args.get('element_token')
            x, y = args.get('x'), args.get('y')
            for element in previous.get('elements', []):
                if 'click' not in element.get('actions', []):
                    continue
                frame = element.get('screenshot_frame', {})
                semantic = bool(token) and token == element.get('element_token')
                pixel = (
                    isinstance(x, (int, float)) and isinstance(y, (int, float))
                    and args.get('capture_id') == previous.get('capture_id')
                    and all(key in frame for key in ('x', 'y', 'w', 'h'))
                    and frame['x'] <= x < frame['x'] + frame['w']
                    and frame['y'] <= y < frame['y'] + frame['h']
                )
                if semantic or pixel:
                    labels.append(element.get('label'))
                    break
        try:
            output = json.loads(call.get('output') or '{}')
        except (ValueError, TypeError):
            continue
        observation = output.get('after_action', output.get('observation', {}))
        if observation.get('capture_id') and observation.get('pid') and observation.get('window_id'):
            windows[(observation['pid'], observation['window_id'])] = observation
    return labels


def calculation_buttons_passed(trace):
    labels = calculation_clicks(trace)
    return any(labels[index:index + 4] == ['6', '*', '7', '=']
               for index in range(len(labels) - 3))
