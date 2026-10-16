package io.github.chriexpe.papo;
import java.io.IOException;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.*;

/** Credential-less native header checks; no WebView or authenticated client. */
final class AppSurfacePolicy {
    static boolean refuses(String xfo, String csp) {
        for (String v : xfo.toLowerCase(Locale.ROOT).split(",")) {
            if (v.trim().equals("deny") || v.trim().equals("sameorigin")) return true;
        }
        for (String policy : csp.toLowerCase(Locale.ROOT).split(",")) for (String directive : policy.split(";")) {
            String[] words = directive.trim().split("\\s+");
            if (words[0].equals("frame-ancestors") && !(words.length == 2 && words[1].equals("*"))) return true;
        }
        return false;
    }
    static boolean sandboxed(String csp) {
        for (String directive : csp.toLowerCase(Locale.ROOT).split(";")) {
            List<String> words = Arrays.asList(directive.trim().split("\\s+"));
            if (words.get(0).equals("sandbox") && !words.contains("allow-same-origin")) return true;
        }
        return false;
    }
    static HttpURLConnection connect(String url, String method) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
        connection.setInstanceFollowRedirects(false);
        connection.setUseCaches(false);
        connection.setConnectTimeout(5000);
        connection.setReadTimeout(5000);
        connection.setRequestMethod(method);
        // This is independent of the native authenticated API client and WebView
        // cookie stores. Do not attach Cookie/Authorization/Referer or tickets.
        connection.connect();
        return connection;
    }
    static String header(HttpURLConnection c, String name) {
        for (Map.Entry<String, List<String>> field : c.getHeaderFields().entrySet()) {
            if (name.equalsIgnoreCase(field.getKey())) return String.join(", ", field.getValue());
        }
        return "";
    }
    static void checkStatus(HttpURLConnection c) throws IOException {
        int status = c.getResponseCode();
        if (status < 200 || status >= 300) throw new IOException("HTTP " + status);
    }
    static void preflight(String url) throws IOException {
        HttpURLConnection connection = connect(url, "HEAD");
        try {
            int status = connection.getResponseCode();
            if (status == 405 || status == 501) {
                connection.disconnect();
                connection = connect(url, "GET");
            }
            checkStatus(connection);
            if (refuses(header(connection, "X-Frame-Options"), header(connection, "Content-Security-Policy"))) throw new IOException("Frame refused");
            // Abort GET fallback after headers, without consuming its body.
        } finally { connection.disconnect(); }
    }
}
