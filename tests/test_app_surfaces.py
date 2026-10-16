"""Native policy regressions that do not require a phone or Mac."""
import importlib.util
import pathlib
import shutil
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("location_policy", ROOT / "scripts/check-macos-location-policy.py")
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class LocationPolicy(unittest.TestCase):
    def test_packaging_has_no_location_authorization(self):
        subprocess.run(["python3", str(ROOT / "scripts/check-macos-location-policy.py"),
                        str(ROOT / "packaging/macos/build-app.sh")], check=True)

    def test_adding_location_usage_keys_is_rejected(self):
        for key in ("NSLocationUsageDescription", "NSLocationWhenInUseUsageDescription",
                    "NSLocationAlwaysAndWhenInUseUsageDescription"):
            with self.assertRaises(ValueError):
                policy.check(["NSCameraUsageDescription", key])
        policy.check(["NSCameraUsageDescription", "NSMicrophoneUsageDescription"])


class AndroidHeaders(unittest.TestCase):
    def test_native_header_policy_against_http_server(self):
        javac = shutil.which("javac")
        java = shutil.which("java")
        self.assertIsNotNone(javac, "JDK required for native policy checks")
        with tempfile.TemporaryDirectory() as folder:
            test = pathlib.Path(folder) / "HeaderTest.java"
            test.write_text(r'''
package io.github.chriexpe.papo;
import com.sun.net.httpserver.HttpServer;
import java.net.InetSocketAddress;
import java.util.*;
import java.io.IOException;
public class HeaderTest {
  static void require(boolean value) { if (!value) throw new AssertionError(); }
  public static void main(String[] args) throws Exception {
    require(AppSurfacePolicy.refuses("SAMEORIGIN", ""));
    require(AppSurfacePolicy.refuses("", "default-src 'self'; frame-ancestors 'self'"));
    require(AppSurfacePolicy.refuses("", "frame-ancestors *; default-src 'self', frame-ancestors 'none'"));
    require(!AppSurfacePolicy.refuses("", "frame-ancestors *"));
    require(!AppSurfacePolicy.refuses("", "default-src 'self'"));
    require(AppSurfacePolicy.sandboxed("sandbox allow-scripts allow-forms"));
    require(!AppSurfacePolicy.sandboxed("sandbox allow-scripts allow-same-origin"));
    HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
    List<String> requests = Collections.synchronizedList(new ArrayList<>());
    server.createContext("/", exchange -> {
      String path = exchange.getRequestURI().getPath();
      String method = exchange.getRequestMethod();
      requests.add(path + ":" + method);
      for (String key : List.of("Cookie", "Authorization", "Referer")) require(exchange.getRequestHeaders().getFirst(key) == null);
      require(exchange.getRequestURI().getFragment() == null);
      int status = 200;
      if ((path.equals("/405") || path.equals("/501")) && method.equals("HEAD")) status = Integer.parseInt(path.substring(1));
      if (path.equals("/redirect")) { status = 302; exchange.getResponseHeaders().set("Location", "/ok"); }
      if (path.equals("/error")) status = 500;
      if (path.equals("/deny")) exchange.getResponseHeaders().set("X-Frame-Options", "DENY");
      if (path.equals("/csp")) {
        exchange.getResponseHeaders().add("Content-Security-Policy", "frame-ancestors *");
        exchange.getResponseHeaders().add("Content-Security-Policy", "frame-ancestors 'none'");
      }
      exchange.sendResponseHeaders(status, -1);
      exchange.close();
    });
    server.start();
    try {
      String base = "http://127.0.0.1:" + server.getAddress().getPort();
      for (String path : List.of("/ok", "/405", "/501")) AppSurfacePolicy.preflight(base + path + "#papo_ticket=not-a-credential");
      require(requests.equals(List.of("/ok:HEAD", "/405:HEAD", "/405:GET", "/501:HEAD", "/501:GET")));
      for (String path : List.of("/redirect", "/error", "/deny", "/csp")) {
        boolean rejected = false;
        try { AppSurfacePolicy.preflight(base + path); } catch (IOException expected) { rejected = true; }
        require(rejected);
      }
      require(!requests.contains("/ok:GET"));
    } finally { server.stop(0); }
  }
}
''')
            source = ROOT / "android/app/src/main/java/io/github/chriexpe/papo/AppSurfacePolicy.java"
            subprocess.run([javac, "-d", folder, str(source), str(test)], check=True)
            subprocess.run([java, "-cp", folder, "io.github.chriexpe.papo.HeaderTest"], check=True, timeout=30)
