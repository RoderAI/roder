import json
import unittest
from browser_smoke import native_receipt_navigation, trace_checks


def row(tool, args=None, state=None, status='completed'):
    return {'tool':tool,'arguments':args or {},'output':json.dumps({'observation':state or {}}),
            'status':status,'error':None,'image_returned':False}

class BrowserProofTests(unittest.TestCase):
    def test_native_browser_chrome_requires_ordered_input_and_later_receipt(self):
        native={'pid':1,'window_id':2}
        prefix=[row('cua_get_window_state',state={**native,'app_name':'Google-chrome'}),
                row('cua_press_key',{**native,'key':'l','modifiers':['ctrl']}),
                row('cua_type_text',{**native,'text':'http://127.0.0.1:8765/receipt'})]
        enter=row('cua_press_key',{**native,'key':'Enter'})
        receipt=row('cua_get_browser_state',state={'page':{'url':'http://127.0.0.1:8765/receipt'},'outline':'Report saved successfully'})
        self.assertTrue(native_receipt_navigation(prefix+[enter,receipt]))
        self.assertFalse(native_receipt_navigation(prefix+[receipt]))
        self.assertFalse(native_receipt_navigation(prefix+[row('cua_press_key',{**native,'key':'Enter'},status='failed'),receipt]))
        self.assertFalse(native_receipt_navigation(prefix+[enter,row('cua_browser_navigate'),receipt]))
        self.assertFalse(native_receipt_navigation(prefix+[row('cua_press_key',{'pid':1,'window_id':3,'key':'Enter'}),receipt]))

    def test_attachment_cannot_claim_same_profile_after_restart_or_copy(self):
        state={'status':'ok','action':'attached_existing_profile','prepared':True,'side_effects':{}}
        self.assertTrue(trace_checks([row('cua_browser_prepare',state=state)])['existing_profile_attached'])
        for effect in ('restarted_browser','copied_profile_data','created_profile','launched_browser'):
            self.assertFalse(trace_checks([row('cua_browser_prepare',state={**state,'side_effects':{effect:True}})])['existing_profile_attached'])

if __name__ == '__main__':unittest.main()
