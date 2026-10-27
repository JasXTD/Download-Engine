## 3.1.1 — 2026-10-27

### Hoàn tất tệp lớn nhanh hơn

Các bản tải HTTP/HTTPS và FTP một phần trên cùng ổ đĩa được xác minh và lưu trực tiếp từ tệp đã tải, không ghi thêm một bản sao đầy đủ. Các bản tải nhiều phần giữ cách ghép bất đồng bộ theo đúng thứ tự; lưu sang ổ đĩa khác vẫn sao chép an toàn. Giữ nguyên kiểm tra checksum, hủy tác vụ, bảo vệ tên tệp trùng và dấu nguồn Internet. Nếu quá trình lưu bị gián đoạn, ứng dụng khôi phục từ phần đã lưu hoặc tệp ghép mà không tải lại hay tạo tệp trùng.
