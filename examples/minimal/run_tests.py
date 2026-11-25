import importlib.util
import sys
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path


def main():
    filename, output = sys.argv[1:]
    sys.path.insert(0, str(Path.cwd()))
    specification = importlib.util.spec_from_file_location("selected_tests", filename)
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    suite = unittest.defaultTestLoader.loadTestsFromModule(module)
    cases = []

    class RecordedResult(unittest.TextTestResult):
        def startTest(self, test):
            cases.append(test)
            super().startTest(test)

    result = unittest.TextTestRunner(resultclass=RecordedResult).run(suite)
    document = ET.Element("testsuite", tests=str(result.testsRun))
    failures = {test.id(): detail for test, detail in result.failures}
    errors = {test.id(): detail for test, detail in result.errors}
    skipped = {test.id(): detail for test, detail in result.skipped}
    for test in cases:
        case = ET.SubElement(
            document, "testcase", name=test._testMethodName, classname=type(test).__name__
        )
        for tag, mapping in (("failure", failures), ("error", errors), ("skipped", skipped)):
            if test.id() in mapping:
                ET.SubElement(case, tag, message=mapping[test.id()]).text = mapping[test.id()]
    Path(output).parent.mkdir(parents=True, exist_ok=True)
    ET.ElementTree(document).write(output, encoding="utf-8", xml_declaration=True)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
