import unittest

from app import bounded_value


class BaselineTest(unittest.TestCase):
    def test_upper_bound(self):
        self.assertEqual(bounded_value(20), 10)
