"""MP-08/MP-10/MP-11: native browser ancestry, never labels or worker liveness."""
import importlib.util
from pathlib import Path
from types import ModuleType, SimpleNamespace
import sys
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('destination',Path(__file__).with_name('native-input-destination.py'))
destination=importlib.util.module_from_spec(spec);spec.loader.exec_module(destination)

class Node:
    def __init__(self,role='push button',widget='',parent=None,pid=42,uri='https://en.wikipedia.org/wiki/Linux'):
        self.role,self.widget,self.parent,self.pid,self.uri=role,widget,parent,pid,uri
    def getRoleName(self):return self.role
    def getAttributes(self):return ['class:'+self.widget]
    def get_process_id(self):return self.pid
    def queryDocument(self):return self
    def getAttributeValue(self,key):return self.uri

class BrowserControl(unittest.TestCase):
    def setUp(self):self.app=Node('application')
    def test_public_navigation_and_page_fields(self):
        for widget in ['OmniboxViewViews','BackButton','ReloadButton','TabCloseButton','WebUIToolbarWebView']:
            destination.browser_control(Node(widget=widget,parent=self.app),42)
        document=Node('document web',parent=self.app)
        # Renderer-controlled class names never classify native action icons.
        destination.browser_control(Node('password text','ToolbarActionView',document),42)
    def test_room_extension_documents_and_nested_ancestors_require_owner(self):
        for uri in ['chrome-extension://abcdefghijkl/options.html', 'chrome://extensions/',
                    'https://chromewebstore.google.com/detail/example',
                    'https://chrome.google.com/webstore/detail/example']:
            with self.subTest(uri=uri):
                document=Node('document web',parent=self.app,uri=uri)
                public_child=Node('document web',parent=document)
                for node in [document,Node(parent=document),Node(parent=public_child)]:
                    with self.assertRaises(destination.NativeExtensionDenied):
                        destination.browser_control(node,42)
    def test_room_public_documents_keep_actions_without_reading_field_values(self):
        for uri in ['https://en.wikipedia.org/wiki/Linux','https://github.com/charioxai/chariox',
                    'chrome://settings/', 'about:blank']:
            destination.browser_control(Node(parent=Node('document web',parent=self.app,uri=uri)),42)
    def test_room_unknown_document_uri_does_not_grant_public_authority(self):
        document=Node('document web',parent=self.app,uri='')
        with self.assertRaises(destination.NativeExtensionDenied):destination.browser_control(document,42)
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

SHOWING, FOCUSED = 1, 2
def rect(x,y,w,h):return SimpleNamespace(x=x,y=y,width=w,height=h)

class At:
    """AT-SPI node in logical (DPR-divided) desktop coordinates."""
    def __init__(self,role='panel',box=(0,0,0,0),widget='',children=(),focused=False,uri='https://en.wikipedia.org/wiki/Linux',pid=42):
        self.role,self.box,self.widget,self.children,self.focused,self.uri,self.pid=role,box,widget,list(children),focused,uri,pid
        self.parent=None
        for child in self.children:child.parent=self
    @property
    def childCount(self):return len(self.children)
    def getChildAtIndex(self,i):return self.children[i]
    def getRoleName(self):return self.role
    def getAttributes(self):return ['class:'+self.widget]
    def get_process_id(self):return self.pid
    def getState(self):return SimpleNamespace(contains=lambda flag:flag==SHOWING or flag==FOCUSED and self.focused)
    def queryDocument(self):return self
    def getAttributeValue(self,key):return self.uri
    def queryComponent(self):return SimpleNamespace(getExtents=lambda coords:rect(*self.box),getAccessibleAtPoint=self.at_point)
    def at_point(self,x,y,coords):
        for child in self.children:
            bx,by,bw,bh=child.box
            if bx<=x<bx+bw and by<=y<by+bh:return child

class XWindow:
    def __init__(self,x,y,w,h):self.box=(x,y,w,h);self.root=None
    def get_geometry(self):return SimpleNamespace(x=self.box[0],y=self.box[1],width=self.box[2],height=self.box[3],border_width=0)
    def query_tree(self):return SimpleNamespace(parent=self.root)
    def query_pointer(self):return SimpleNamespace(child=None)

def native_accessibility():
    atspi=ModuleType('pyatspi');atspi.STATE_SHOWING=SHOWING;atspi.STATE_FOCUSED=FOCUSED;atspi.DESKTOP_COORDS=0;atspi.ROLE_PASSWORD_TEXT=-1
    with patch.dict(sys.modules,{'pyatspi':atspi}):
        spec=importlib.util.spec_from_file_location('accessibility',Path(__file__).with_name('native-accessibility.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    module.alive=lambda process:True;module.pyatspi=atspi
    return module,atspi

class DeviceScale(unittest.TestCase):
    """Physical X11 input at DPR 2 against logical AT-SPI extents, two windows."""
    def setUp(self):
        self.accessibility,self.atspi=native_accessibility()
        self.root=SimpleNamespace(translate_coords=lambda window,x,y:SimpleNamespace(x=window.box[0]+x,y=window.box[1]+y),id=1)
        self.pointer=SimpleNamespace(root_x=0,root_y=0,child=None,mask=0)
        self.root.query_pointer=lambda:self.pointer
        self.focus=None
        self.connection=SimpleNamespace(screen=lambda:SimpleNamespace(root=self.root),get_input_focus=lambda:SimpleNamespace(focus=self.focus))
        self.process={'pid':42,'browser':True}
        self.omnibox=At('entry',(100,10,500,30),'OmniboxViewViews',focused=True)
        self.field=At('entry',(600,500,100,30),focused=True)
        self.page_a=At('document web',(0,100,1000,700),children=[self.field])
        self.frame_a=At('frame',(0,0,1000,800),children=[At('tool bar',(0,0,1000,100),children=[self.omnibox]),self.page_a])
        self.window_a=self.x_window(0,0,2000,1600)
    def x_window(self,x,y,w,h):
        window=XWindow(x,y,w,h);window.root=self.root;return window
    def install(self,*frames):
        app=At('application',children=frames,pid=42)
        desktop=At('desktop',children=[app],pid=0)
        self.atspi.Registry=SimpleNamespace(getDesktop=lambda index:desktop)
    def check(self,point=None):
        if point is not None:
            self.pointer.root_x,self.pointer.root_y,self.pointer.child=point[0],point[1],self.window_a
        else:self.focus=self.focus or self.window_a
        with patch.object(destination,'owner_pid',lambda connection,window:42):
            destination.input_destination(self.connection,[self.process],self.accessibility,point)
    def test_dpr2_clicks_in_every_quadrant_convert_the_hit_test_point(self):
        self.install(self.frame_a)
        self.check((1250,1020))   # bottom-right page field: logical (625,510)
        self.check((300,40))      # omnibox
    def test_dpr2_click_on_extension_action_is_still_denied_after_conversion(self):
        action=At('push button',(900,10,40,40),'ToolbarActionView')
        self.frame_a.children[0].children.append(action);action.parent=self.frame_a.children[0]
        self.install(self.frame_a)
        with self.assertRaises(destination.NativeExtensionDenied):self.check((1820,40))
    def test_point_outside_every_bound_window_is_unavailable(self):
        self.install(At('frame',(2000,0,1000,800),children=[]))
        with self.assertRaises(destination.NativeExtensionDenied):self.check((1250,1020))
    def two_windows(self,focus_second=False,restricted_focus=False):
        extensions=At('document web',(1500,100,1000,700),uri='chrome://extensions/',focused=restricted_focus)
        frame_b=At('frame',(1500,0,1000,800),children=[extensions])
        self.install(self.frame_a,frame_b)
        if focus_second:self.focus=self.x_window(3000,0,2000,1600)
    def test_keyboard_ignores_restricted_document_in_an_unfocused_window(self):
        self.two_windows();self.check()
    def test_keyboard_ignores_restricted_document_in_the_same_window_when_unfocused(self):
        self.frame_a.children.append(At('document web',(0,100,10,10),uri='chrome-extension://abcdefghijkl/popup.html'))
        self.install(self.frame_a);self.check()
    def test_keyboard_denies_a_focused_restricted_document(self):
        self.two_windows(focus_second=True,restricted_focus=True)
        with self.assertRaises(destination.NativeExtensionDenied):self.check()
    def test_identical_geometry_windows_are_all_checked(self):
        other=At('frame',(0,0,1000,800),children=[At('document web',(0,100,1000,700),uri='chrome://extensions/',children=[At('entry',(600,500,100,30))])])
        self.install(self.frame_a,other)
        self.check()
        with self.assertRaises(destination.NativeExtensionDenied):self.check((1250,1020))
    def test_keyboard_denies_unbound_window(self):
        self.install(At('frame',(1500,0,1000,800)))
        with self.assertRaises(destination.NativeExtensionDenied):self.check()

if __name__=='__main__':unittest.main()
