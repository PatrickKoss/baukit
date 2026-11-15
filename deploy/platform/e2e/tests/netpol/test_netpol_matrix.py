import unittest

import netpol_matrix

ROW = """\
- from: {namespace: netpol-tenant, labels: {}}
  to: {service: loki-gateway.observability, port: 80}
  expect: deny
"""


class ParseTests(unittest.TestCase):
    def parse_error(self, text):
        with self.assertRaises(netpol_matrix.MatrixError) as raised:
            netpol_matrix.parse(text)
        return str(raised.exception)

    def test_parses_each_destination_kind(self):
        rows = netpol_matrix.parse(
            ROW + "- from: {namespace: a, labels: {x: w}}\n"
            "  to: {pod: {namespace: b, labels: {app: z}}, port: 8080}\n"
            "  expect: allow\n"
            "- from: {namespace: a}\n"
            "  to: {host: node, port: 6443}\n"
            "  expect: deny\n"
        )
        self.assertEqual([row.number for row in rows], [1, 2, 3])
        self.assertEqual(rows[0].kind, "service")
        self.assertEqual(rows[0].destination, "loki-gateway.observability:80")
        self.assertEqual(rows[1].pod_labels, {"app": "z"})
        self.assertEqual(rows[1].describe(), "#2 a {x=w} -> pod b/app=z:8080 expect allow")
        self.assertEqual((rows[2].kind, rows[2].target, rows[2].labels), ("host", "node", {}))

    def test_rejects_an_unknown_verdict(self):
        self.assertIn("expect must be allow or deny", self.parse_error(ROW.replace("deny", "drop")))

    def test_rejects_two_destinations(self):
        text = ROW.replace("port: 80}", "host: 1.1.1.1, port: 80}")
        self.assertIn("exactly one of", self.parse_error(text))

    def test_rejects_a_service_without_namespace(self):
        text = ROW.replace("loki-gateway.observability", "loki-gateway")
        self.assertIn("<name>.<namespace>", self.parse_error(text))

    def test_rejects_a_host_name(self):
        text = ROW.replace("service: loki-gateway.observability", "host: example.com")
        self.assertIn("IPv4 address or node", self.parse_error(text))

    def test_rejects_a_port_out_of_range(self):
        self.assertIn("TCP port", self.parse_error(ROW.replace("port: 80", "port: 70000")))

    def test_rejects_an_empty_matrix(self):
        self.assertIn("non-empty list", self.parse_error("# nothing\n"))

    def test_the_checked_in_matrices_parse(self):
        for name in ("matrix.yaml", "posthog-matrix.yaml"):
            with self.subTest(name=name):
                self.assertTrue(netpol_matrix.load(netpol_matrix.Path(__file__).parent / name))


if __name__ == "__main__":
    unittest.main()
