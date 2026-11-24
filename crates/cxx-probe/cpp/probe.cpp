// Spike probe: Eigen (header-only) + std::thread + exceptions, C ABI.
#include <Eigen/Dense>
#include <thread>
#include <stdexcept>

extern "C" float cxx_probe(int n) {
    if (n < 1) n = 1;
    if (n > 64) n = 64;
    float result = 0.0f;
    try {
        // Eigen matrix product on a worker thread (libc++ std::thread on iOS).
        std::thread t([&] {
            Eigen::MatrixXf a = Eigen::MatrixXf::Constant(64, 64, 1.0f);
            Eigen::MatrixXf b = Eigen::MatrixXf::Identity(64, 64) * 2.0f;
            Eigen::MatrixXf c = a * b;           // every entry = 2
            result = c.topLeftCorner(n, n).sum(); // 2 * n * n
            if (result < 0) throw std::runtime_error("unreachable");
        });
        t.join();
    } catch (const std::exception&) {
        return -1.0f;
    }
    return result;
}
