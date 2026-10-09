import json
import unittest
from calculator_trace import calculation_buttons_passed, calculation_clicks


def observation(index, label):
    return {'pid': 10, 'window_id': 20, 'capture_id': f'capture-{index}',
            'elements': [{'label': label, 'actions': ['click'],
                          'element_token': f'bound-{index}:0',
                          'screenshot_frame': {'x': 10, 'y': 20, 'w': 30, 'h': 40}}]}


def sequence():
    result = []
    for index, label in enumerate(['6', '*', '7', '=']):
        state = observation(index, label)
        result.append({'tool': 'cua_get_window_state', 'status': 'completed',
                       'output': json.dumps({'observation': state})})
        args = {'pid': 10, 'window_id': 20}
        args.update({'x': 15, 'y': 25, 'capture_id': state['capture_id']} if index == 0
                    else {'element_token': state['elements'][0]['element_token']})
        result.append({'tool': 'cua_click', 'status': 'completed', 'arguments': args})
    return result


class CalculatorTraceTests(unittest.TestCase):
    def test_actual_pixel_and_semantic_button_sequence_passes(self):
        self.assertEqual(calculation_clicks(sequence()), ['6', '*', '7', '='])
        self.assertTrue(calculation_buttons_passed(sequence()))

    def test_macos_ax_labels_and_null_after_action_preserve_sequence(self):
        trace = sequence()
        for index, label in enumerate(['6', 'Multiply', '7', 'Equals']):
            state = observation(index, label)
            state['elements'][0]['actions'] = ['AXPress']
            trace[index * 2]['output'] = json.dumps({'observation':state, 'after_action':None})
        self.assertEqual(calculation_clicks(trace), ['6', '*', '7', '='])
        self.assertTrue(calculation_buttons_passed(trace))

    def test_keypress_operand_does_not_prove_button_click(self):
        trace = sequence()
        trace[5]['tool'] = 'cua_press_key'
        self.assertFalse(calculation_buttons_passed(trace))

    def test_failed_foreign_or_stale_click_does_not_count(self):
        for arguments, error in [
                ({'pid': 10, 'window_id': 20, 'x': 15, 'y': 25, 'capture_id': 'old'}, None),
                ({'pid': 11, 'window_id': 20, 'element_token': 'bound-0:0'}, None),
                ({'pid': 10, 'window_id': 20, 'element_token': 'bound-0:0'}, 'refused')]:
            with self.subTest(arguments=arguments, error=error):
                trace = sequence()
                trace[1].update(arguments=arguments, error=error)
                self.assertFalse(calculation_buttons_passed(trace))


if __name__ == '__main__':
    unittest.main()
