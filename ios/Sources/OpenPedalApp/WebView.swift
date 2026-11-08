import SwiftUI
import WebKit

struct WebView: UIViewRepresentable {
    let url: URL

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: WKWebViewConfiguration())
        view.isOpaque = false
        view.backgroundColor = UIColor(red: 0.086, green: 0.086, blue: 0.086, alpha: 1)
        view.scrollView.bounces = false
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}
}
