#pragma once

#include <cstddef>
#include <cstdint>
#include <iosfwd>
#include <memory>
#include <string>
#include <string_view>

namespace hexal {

// A piece table over two immutable-by-convention buffers:
//
//   * the original file contents (never modified), and
//   * an append-only "add" buffer holding all text inserted by the user.
//
// The piece list is stored in an implicit treap. Every node is augmented with
// the number of bytes and newline characters in its subtree, which makes
// offset -> line and line -> offset queries O(log n) without rescanning the
// document. Splitting a piece inside a treap node creates two nodes that keep
// the original priority, preserving the heap invariant.
//
// This is the buffer strategy required by the specification: ordinary edits
// allocate memory proportional to the edited text, never to the file size.
class PieceTable {
public:
    PieceTable();
    ~PieceTable();

    PieceTable(PieceTable&&) noexcept;
    PieceTable& operator=(PieceTable&&) noexcept;

    PieceTable(const PieceTable&) = delete;
    PieceTable& operator=(const PieceTable&) = delete;

    // Replaces the document with a new original buffer (used when loading).
    void setOriginal(std::string original);

    std::size_t size() const noexcept;
    std::size_t newlineCount() const noexcept;

    void insert(std::size_t offset, std::string_view text);
    void erase(std::size_t offset, std::size_t length);

    std::string toString() const;
    std::string range(std::size_t offset, std::size_t length) const;
    char byteAt(std::size_t offset) const;

    // Streams the whole document in order. Used by atomic save so that saving
    // does not require a second full-size copy of the document.
    bool writeTo(std::ostream& out) const;

    // Byte offset of the first character of `line`. `lineCount()` is obtained
    // by adding one to newlineCount(). Out-of-range lines are clamped to size.
    std::size_t lineStart(std::size_t line) const;

    // 0-based line containing `offset`.
    std::size_t lineOfOffset(std::size_t offset) const;

private:
    struct Node;

    using NodePtr = std::unique_ptr<Node>;

    static std::size_t bytesOf(const Node* node) noexcept;
    static std::size_t newlinesOf(const Node* node) noexcept;
    static void update(Node* node) noexcept;

    NodePtr merge(NodePtr left, NodePtr right);
    void split(NodePtr tree, std::size_t offset, NodePtr& left, NodePtr& right);

    static std::size_t countNewlines(std::string_view text, std::size_t length) noexcept;
    static std::size_t newlineOffsetInPiece(const Node* node, std::string_view source,
                                            std::size_t nth) noexcept;

    NodePtr makeNode(bool fromAddBuffer, std::size_t start, std::size_t length);
    NodePtr makeTreeForAdded(std::size_t start, std::size_t length);
    std::size_t chooseChunkEnd(const std::string& source, std::size_t start,
                               std::size_t limit) const;

    const std::string& sourceFor(const Node* node) const noexcept;

    bool writeNode(const Node* node, std::ostream& out) const;
    void collectRange(const Node* node, std::size_t base, std::size_t begin,
                      std::size_t end, std::string& out) const;

    std::uint64_t nextPriority() noexcept;

    std::string original_;
    std::string added_;
    NodePtr root_;
    std::uint64_t randomState_ = 0x9E3779B97F4A7C15ULL;
};

} // namespace hexal
