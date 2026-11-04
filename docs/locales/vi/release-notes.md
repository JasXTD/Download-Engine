## 3.10.1 — 2026-11-04

Cải thiện giới hạn tài nguyên khi tìm kiếm peer torrent, xử lý hủy và lọc địa chỉ mạng. Cập nhật kho thông tin xác thực trên macOS và Linux nhưng vẫn giữ thông tin đăng nhập hiện có, đồng thời ngăn ghi đè đích đã tồn tại khi lưu thư mục torrent.

[English details](../../../CHANGELOG.md#3101--2026-11-04)

## 3.10.0 — 2026-11-03

### macOS và Linux

Thêm gói gốc cho Mac Apple Silicon và Intel cùng tệp nén Linux x64, dùng chung giao diện và bộ máy tải xuống với Windows. Bao gồm FFmpeg, mã nguồn và giấy phép, hộp thoại gốc và kho thông tin xác thực của hệ điều hành. Gói Mac được ký ad hoc, chưa được Apple công chứng. Xem hướng dẫn về yêu cầu và phạm vi kiểm thử.

[Guide / English](../../cross-platform.md)

## 3.9.0 — 2026-11-01

### Quy trình torrent

Hỗ trợ liên kết magnet từ trình duyệt, kéo thả tệp torrent và gợi ý từ bảng nhớ tạm. Chia sẻ sau khi hoàn tất và kết nối đến là hai tùy chọn riêng, mặc định tắt. Có thể quét nhiều tệp, di chuyển hoặc đổi tên thư mục; Dừng tất cả cũng dừng chia sẻ.

[Guide / English](../../user-guide.md#download-torrents)

## 3.8.0 — 2026-11-01

### Mở torrent từ Windows

Mở tệp .torrent từ Explorer hoặc liên kết magnet từ trình duyệt để xem xét. Đăng ký với Windows là tùy chọn, giữ nguyên ứng dụng mặc định và giữ các mục chờ sau khi khởi động lại. Tăng cường bảo vệ torrent; magnet chưa có siêu dữ liệu trong bộ nhớ đệm cần tracker, nếu không hãy dùng tệp .torrent. Đã thêm kiểm thử hồi quy cho cơ chế chống xung đột tên tệp hiện có.

## 3.7.0 — 2026-11-01

### Tải torrent

Hỗ trợ tệp torrent và liên kết magnet, chọn tệp, tạm dừng, tiếp tục và khôi phục phần đã xác minh. Mặc định tắt; bật và quản lý tải lên trong Tùy chọn. Torrent tạm dừng khi bật proxy và ngừng tải lên khi hoàn tất. Bao gồm các bản sửa lỗi 3.6.1 và trình trợ giúp torrent trong gói cài đặt.

### Tích hợp bản phát hành

Đã cập nhật hướng dẫn, README, trang web và kiểm tra trình cài đặt. Sau khi thêm torrent đầu tiên, hồ sơ yêu cầu phiên bản 3.7.0 trở lên để ngăn phiên bản cũ sửa tác vụ torrent. Chỉ bật tính năng không nâng cấp hồ sơ.

## 3.6.1 — 2026-10-31

### Tinh gọn phụ thuộc và phê duyệt tiện ích

Thay fs2 bằng khóa tệp tích hợp của Rust và loại bỏ phụ thuộc winapi cũ, đồng thời giữ nguyên cách khóa và thời gian chờ. Bản cập nhật tiện ích thêm tác vụ, kể cả thay ID tác vụ, nay cần được phê duyệt ngay cả khi tiện ích đang tắt.

## 3.6.0 — 2026-10-31

### Tiện ích mở rộng

Trang Tiện ích mở rộng mới thay thế Tập lệnh, gồm các thẻ Đã cài đặt, Tập lệnh, Quy tắc và Lịch sử chạy. Mỗi tiện ích mở rộng là một tệp .de-extension.json gộp trình đọc trang, nhà cung cấp tải xuống và tác vụ tự động hóa. Chỉ cài đặt từ tệp cục bộ; nhà phát hành chưa được xác minh. Cửa sổ xem xét cho biết tiện ích mở rộng làm được gì, sau khi cài đặt nó ở trạng thái tắt, và bản cập nhật phải hỏi trước khi nhận quyền truy cập mới. Truy cập web chỉ giới hạn ở các máy chủ đã khai báo. Sau khi cài tiện ích mở rộng đầu tiên, ứng dụng cũ không thể ghi vào hồ sơ này.

## 3.5.2 — 2026-10-30

### Kiểm tra cập nhật khi mở ứng dụng

Bật theo mặc định và kiểm tra một lần mỗi khi mở ứng dụng. Khi có phiên bản mới, bạn chọn tải bản cập nhật hoặc để sau. Lỗi ngoại tuyến không mở hộp thoại. Có thể tắt trong Tùy chọn. Tải xuống và cài đặt bản cập nhật từ trang phát hành GitHub chính thức.

## 3.5.1 — 2026-10-29

### Bảo mật trình cung cấp Lua

- Resolve/refresh và transform/finish chạy trong các tiến trình có quyền riêng biệt. Tiến trình xử lý không có `xd.fetch`, kể cả khi nạp mã. Dùng `xd.phase` để khởi tạo theo giai đoạn; biến cục bộ của trình phân giải không được chuyển sang xử lý.
- Kế hoạch đã lưu chỉ giữ kích thước, mã băm và định danh phép biến đổi. URL đã phân giải được làm mới khi cần; bản mã nguồn cố định của tác vụ được bảo vệ bằng DPAPI của người dùng Windows hiện tại. Tác vụ cũ được nâng cấp nguyên tử, giữ liên kết điểm kiểm tra và chặn phiên bản cũ ghi dữ liệu.
- `xd.key` và `scripts protect-key`, đọc khóa từ stdin được chuyển hướng, cung cấp handle khóa AES không lộ nội dung và gắn với trình cung cấp. Không cần đưa khóa bí mật vào mã hay bản xuất. Bản sao lưu cũ và khóa viết trực tiếp trong thư viện không bị xóa; xem `docs/lua-providers.md`.

### Xóa lịch sử

Xóa lịch sử đã hoàn tất nay yêu cầu xác nhận và hiển thị hàng đợi cùng số lượng. Hủy hoặc đóng để giữ lịch sử. Các tệp đã tải xuống luôn được giữ lại.

## 3.5.0 — 2026-10-29

### Lua Provider API 1

Nhà cung cấp tải xuống Lua hỗ trợ xác định nguồn, xử lý byte, xác minh mã băm và tạm dừng/tiếp tục. Mỗi tác vụ giữ nguyên phiên bản mã đã lưu. Chỉ kiểm thử bằng tệp mã hóa được tạo cục bộ. Sau khi lưu nhà cung cấp, ứng dụng cũ không thể ghi vào hồ sơ này.

## 3.4.1 — 2026-10-29

Danh sách tải xuống giảm sao chép bản ghi và tìm kiếm lặp lại trong hàng đợi lớn. Cửa sổ nhỏ dành thêm chỗ cho tên tệp. Bắt đầu hoặc tạm dừng hàng đợi cạnh trạng thái; bắt đầu sẽ ghi đè lịch. Di chuột lên trạng thái để xem giải thích thử lại và lỗi. Truyền dữ liệu, xác minh và định dạng tiếp tục tải không thay đổi.

## 3.4.0 — 2026-10-29

### Bộ sưu tập giao diện

Tùy chọn → Giao diện → Tùy chỉnh có Neon Courier, Reactor Night, Aurora và Moonveil. Điều chỉnh nền và màu nhấn hoặc nhập PNG/JPEG. Lưu giữ bản xem trước; Hủy khôi phục giao diện cũ. Ảnh chỉ lưu cục bộ, có giới hạn kích thước và được xử lý nền.

### Cải tiến hồ sơ và điều khiển

Bản này cũng có bảo vệ phiên bản hồ sơ và kiểm soát truy cập MCP.
