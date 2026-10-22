#!/usr/bin/env python3
# MP-08/MP-10: real compiler log shapes, including unknown file context.
import importlib.util,pathlib,unittest
spec=importlib.util.spec_from_file_location('errors',pathlib.Path(__file__).with_name('parse-errors.py'));mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
class Errors(unittest.TestCase):
 def test_tectonic_and_repeated_tex_tail(self):
  text='error: main.tex:3: Undefined control sequence\n! Undefined control sequence.\nl.3 \\bad\n'
  self.assertEqual(mod.parse(text),[{'file':'main.tex','line':3,'message':'Undefined control sequence'}])
 def test_real_component_stderr(self):
  text='[wasm-rust-async] tectonic error: main.tex:3: Undefined control sequence\n[wasm-rust-async] fatal: longjmp called (TeX engine abort)'
  self.assertEqual(mod.parse(text),[{'file':'main.tex','line':3,'message':'Undefined control sequence'}])
 def test_included_file(self):
  self.assertEqual(mod.parse('error: sections/results.tex:27: Missing $ inserted')[0]['file'],'sections/results.tex')
 def test_no_guessed_filename(self):
  self.assertEqual(mod.parse('! Missing $ inserted.\nl.19 x_2')[0],{'file':None,'line':19,'message':'Missing $ inserted.'})
if __name__=='__main__':unittest.main()
