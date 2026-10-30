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
