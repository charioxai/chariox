"""MP-08/MP-10/MP-11: native browser ancestry, never labels or worker liveness."""
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('destination',Path(__file__).with_name('native-input-destination.py'))
destination=importlib.util.module_from_spec(spec);spec.loader.exec_module(destination)

class Node:
    def __init__(self,role='push button',widget='',parent=None,pid=42):
        self.role,self.widget,self.parent,self.pid=role,widget,parent,pid
    def getRoleName(self):return self.role
    def getAttributes(self):return ['class:'+self.widget]
    def get_process_id(self):return self.pid

class BrowserControl(unittest.TestCase):
    def setUp(self):self.app=Node('application')
    def test_public_navigation_and_page_fields(self):
        for widget in ['OmniboxViewViews','BackButton','ReloadButton','TabCloseButton','WebUIToolbarWebView']:
            destination.browser_control(Node(widget=widget,parent=self.app),42)
        document=Node('document web',parent=self.app)
        # Renderer-controlled class names never classify native action icons.
        destination.browser_control(Node('password text','ToolbarActionView',document),42)
    def test_extension_ancestors_win_over_public_looking_children(self):
        for widget in ['ToolbarActionView','ExtensionsToolbarButton','ExtensionsMenuItemView']:
            extension=Node(widget=widget,parent=self.app)
            for node in [extension,Node(widget='ToolbarButton',parent=extension)]:
                with self.assertRaises(destination.NativeExtensionDenied):destination.browser_control(node,42)
    def test_unknown_missing_foreign_and_unbounded_ancestry_refuse(self):
        for node in [None,Node(parent=self.app),Node(widget='ToolbarButton',parent=self.app,pid=43)]:
            with self.assertRaises(destination.NativeExtensionDenied):destination.browser_control(node,42)
        cycle=Node(widget='ToolbarButton');cycle.parent=cycle
        with self.assertRaises(destination.NativeExtensionDenied):destination.browser_control(cycle,42)

if __name__=='__main__':unittest.main()
