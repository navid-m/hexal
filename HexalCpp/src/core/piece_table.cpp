#include "core/piece_table.hpp"

#include <algorithm>
#include <ostream>

namespace hexal {
namespace {

constexpr std::size_t kChunkSize = 8192;

} // namespace

struct PieceTable::Node {
    bool fromAddBuffer = false;
    std::size_t start = 0;
    std::size_t length = 0;

    // Number of newline bytes inside this piece itself.
    std::size_t pieceNewlines = 0;

    // Augmented subtree values.
    std::size_t bytes = 0;
    std::size_t newlines = 0;

    std::uint64_t priority = 0;

    NodePtr left;
    NodePtr right;
};

PieceTable::PieceTable() = default;
PieceTable::~PieceTable() = default;
PieceTable::PieceTable(PieceTable&&) noexcept = default;
PieceTable& PieceTable::operator=(PieceTable&&) noexcept = default;

std::size_t PieceTable::bytesOf(const Node* node) noexcept {
    return node ? node->bytes : 0;
}

std::size_t PieceTable::newlinesOf(const Node* node) noexcept {
    return node ? node->newlines : 0;
}

void PieceTable::update(Node* node) noexcept {
    if (!node) {
        return;
    }
    node->bytes = node->length + bytesOf(node->left.get()) + bytesOf(node->right.get());
    node->newlines = node->pieceNewlines + newlinesOf(node->left.get()) +
                     newlinesOf(node->right.get());
}

const std::string& PieceTable::sourceFor(const Node* node) const noexcept {
    return node->fromAddBuffer ? added_ : *original_;
}

std::size_t PieceTable::countNewlines(std::string_view text, std::size_t length) noexcept {
    length = std::min(length, text.size());
    return static_cast<std::size_t>(
        std::count(text.begin(), text.begin() + static_cast<std::ptrdiff_t>(length), '\n'));
}

std::size_t PieceTable::newlineOffsetInPiece(const Node* node, std::string_view source,
                                             std::size_t nth) noexcept {
    const std::size_t end = node->start + node->length;
    std::size_t seen = 0;
    for (std::size_t i = node->start; i < end; ++i) {
        if (source[i] == '\n') {
            ++seen;
            if (seen == nth) {
                return i - node->start;
            }
        }
    }
    return node->length;
}

std::uint64_t PieceTable::nextPriority() noexcept {
    // splitmix64: deterministic and well distributed.
    randomState_ += 0x9E3779B97F4A7C15ULL;
    std::uint64_t z = randomState_;
    z = (z ^ (z >> 30U)) * 0xBF58476D1CE4E5B9ULL;
    z = (z ^ (z >> 27U)) * 0x94D049BB133111EBULL;
    return z ^ (z >> 31U);
}

PieceTable::NodePtr PieceTable::makeNode(bool fromAddBuffer, std::size_t start,
                                         std::size_t length) {
    auto node = std::make_unique<Node>();
    node->fromAddBuffer = fromAddBuffer;
    node->start = start;
    node->length = length;
    node->priority = nextPriority();

    const std::string& source = fromAddBuffer ? added_ : *original_;
    node->pieceNewlines =
        countNewlines(std::string_view(source).substr(start, length), length);

    update(node.get());
    return node;
}

PieceTable::NodePtr PieceTable::merge(NodePtr left, NodePtr right) {
    if (!left) {
        return right;
    }
    if (!right) {
        return left;
    }

    if (left->priority >= right->priority) {
        left->right = merge(std::move(left->right), std::move(right));
        update(left.get());
        return left;
    }

    right->left = merge(std::move(left), std::move(right->left));
    update(right.get());
    return right;
}

void PieceTable::split(NodePtr tree, std::size_t offset, NodePtr& left, NodePtr& right) {
    if (!tree) {
        left.reset();
        right.reset();
        return;
    }

    const std::size_t leftBytes = bytesOf(tree->left.get());

    if (offset < leftBytes) {
        NodePtr lhs;
        NodePtr rhs;
        split(std::move(tree->left), offset, lhs, rhs);
        tree->left = std::move(rhs);
        update(tree.get());
        left = std::move(lhs);
        right = std::move(tree);
        return;
    }

    if (offset > leftBytes + tree->length) {
        NodePtr lhs;
        NodePtr rhs;
        split(std::move(tree->right), offset - leftBytes - tree->length, lhs, rhs);
        tree->right = std::move(lhs);
        update(tree.get());
        left = std::move(tree);
        right = std::move(rhs);
        return;
    }

    if (offset == leftBytes) {
        left = std::move(tree->left);
        tree->left.reset();
        update(tree.get());
        right = std::move(tree);
        return;
    }

    if (offset == leftBytes + tree->length) {
        right = std::move(tree->right);
        tree->right.reset();
        update(tree.get());
        left = std::move(tree);
        return;
    }

    // The split point is inside this piece. Keep the original priority on both
    // halves so that the heap property relative to the existing children is
    // preserved.
    const std::size_t localOffset = offset - leftBytes;

    auto lhs = std::make_unique<Node>();
    lhs->fromAddBuffer = tree->fromAddBuffer;
    lhs->start = tree->start;
    lhs->length = localOffset;
    lhs->priority = tree->priority;
    lhs->left = std::move(tree->left);
    {
        const std::string& source = sourceFor(lhs.get());
        lhs->pieceNewlines =
            countNewlines(std::string_view(source).substr(lhs->start, lhs->length),
                          lhs->length);
    }
    update(lhs.get());

    auto rhs = std::make_unique<Node>();
    rhs->fromAddBuffer = tree->fromAddBuffer;
    rhs->start = tree->start + localOffset;
    rhs->length = tree->length - localOffset;
    rhs->priority = tree->priority;
    rhs->right = std::move(tree->right);
    {
        const std::string& source = sourceFor(rhs.get());
        rhs->pieceNewlines =
            countNewlines(std::string_view(source).substr(rhs->start, rhs->length),
                          rhs->length);
    }
    update(rhs.get());

    left = std::move(lhs);
    right = std::move(rhs);
}

std::size_t PieceTable::chooseChunkEnd(const std::string& source, std::size_t start,
                                       std::size_t limit) const {
    if (start >= limit) {
        return limit;
    }

    const std::size_t maxEnd = std::min(limit, start + kChunkSize);
    if (maxEnd < limit) {
        const std::size_t newline = source.rfind('\n', maxEnd - 1);
        if (newline != std::string::npos && newline >= start) {
            return newline + 1;
        }
    }
    return maxEnd;
}

PieceTable::NodePtr PieceTable::makeTreeForAdded(std::size_t start, std::size_t limit) {
    NodePtr tree;
    std::size_t pos = start;
    while (pos < limit) {
        const std::size_t end = chooseChunkEnd(added_, pos, limit);
        tree = merge(std::move(tree), makeNode(true, pos, end - pos));
        pos = end;
    }
    return tree;
}

void PieceTable::setOriginal(std::string original) {
    original_ = std::make_shared<const std::string>(std::move(original));
    added_.clear();
    root_.reset();

    std::size_t pos = 0;
    while (pos < original_->size()) {
        std::size_t end = chooseChunkEnd(*original_, pos, original_->size());
        if (end <= pos) {
            end = original_->size();
        }
        root_ = merge(std::move(root_), makeNode(false, pos, end - pos));
        pos = end;
    }
}

std::size_t PieceTable::size() const noexcept {
    return bytesOf(root_.get());
}

std::size_t PieceTable::newlineCount() const noexcept {
    return newlinesOf(root_.get());
}

void PieceTable::insert(std::size_t offset, std::string_view text) {
    if (text.empty()) {
        return;
    }

    offset = std::min(offset, size());

    const std::size_t addStart = added_.size();
    added_.append(text.data(), text.size());

    NodePtr inserted = makeTreeForAdded(addStart, added_.size());

    NodePtr left;
    NodePtr right;
    split(std::move(root_), offset, left, right);
    root_ = merge(merge(std::move(left), std::move(inserted)), std::move(right));
}

void PieceTable::erase(std::size_t offset, std::size_t length) {
    if (length == 0 || offset >= size()) {
        return;
    }

    length = std::min(length, size() - offset);

    NodePtr left;
    NodePtr rest;
    split(std::move(root_), offset, left, rest);

    NodePtr removed;
    NodePtr right;
    split(std::move(rest), length, removed, right);

    // `removed` goes out of scope here, freeing the discarded pieces.
    root_ = merge(std::move(left), std::move(right));
}

void PieceTable::collectRange(const Node* node, std::size_t base, std::size_t begin,
                              std::size_t end, std::string& out) const {
    if (!node || begin >= end) {
        return;
    }

    const std::size_t leftBytes = bytesOf(node->left.get());
    const std::size_t nodeStart = base + leftBytes;
    const std::size_t nodeEnd = nodeStart + node->length;

    if (begin < nodeStart) {
        collectRange(node->left.get(), base, begin, std::min(end, nodeStart), out);
    }

    if (begin < nodeEnd && end > nodeStart) {
        const std::size_t from = std::max(begin, nodeStart) - nodeStart;
        const std::size_t to = std::min(end, nodeEnd) - nodeStart;
        const std::string& source = sourceFor(node);
        out.append(source, node->start + from, to - from);
    }

    if (end > nodeEnd) {
        collectRange(node->right.get(), nodeEnd, std::max(begin, nodeEnd), end, out);
    }
}

std::string PieceTable::range(std::size_t offset, std::size_t length) const {
    const std::size_t total = size();
    if (offset >= total || length == 0) {
        return {};
    }

    length = std::min(length, total - offset);
    std::string result;
    result.reserve(length);
    collectRange(root_.get(), 0, offset, offset + length, result);
    return result;
}

std::string PieceTable::toString() const {
    return range(0, size());
}

char PieceTable::byteAt(std::size_t offset) const {
    if (offset >= size()) {
        return '\0';
    }

    std::size_t base = 0;
    const Node* node = root_.get();
    while (node) {
        const std::size_t leftBytes = bytesOf(node->left.get());
        const std::size_t nodeStart = base + leftBytes;
        if (offset < nodeStart) {
            node = node->left.get();
            continue;
        }
        if (offset < nodeStart + node->length) {
            return sourceFor(node)[node->start + (offset - nodeStart)];
        }
        base = nodeStart + node->length;
        node = node->right.get();
    }
    return '\0';
}

bool PieceTable::writeNode(const Node* node, std::ostream& out) const {
    if (!node) {
        return true;
    }

    if (!writeNode(node->left.get(), out)) {
        return false;
    }

    const std::string& source = sourceFor(node);
    out.write(source.data() + static_cast<std::ptrdiff_t>(node->start),
              static_cast<std::streamsize>(node->length));
    if (!out) {
        return false;
    }

    return writeNode(node->right.get(), out);
}

bool PieceTable::writeTo(std::ostream& out) const {
    return writeNode(root_.get(), out);
}

bool PieceTable::Snapshot::writeTo(std::ostream& out) const {
    for (const Ref& ref : pieces_) {
        const std::string& source = ref.fromAddBuffer ? added_ : *original_;
        out.write(source.data() + static_cast<std::ptrdiff_t>(ref.start),
                  static_cast<std::streamsize>(ref.length));
        if (!out) {
            return false;
        }
    }
    return true;
}

PieceTable::Snapshot PieceTable::snapshot() const {
    Snapshot result;
    result.original_ = original_;
    result.added_ = added_;
    collectSnapshot(root_.get(), result.pieces_);
    return result;
}

void PieceTable::collectSnapshot(const Node* node,
                                 std::vector<Snapshot::Ref>& pieces) const {
    if (!node) {
        return;
    }

    collectSnapshot(node->left.get(), pieces);
    pieces.push_back(Snapshot::Ref{node->fromAddBuffer, node->start, node->length});
    collectSnapshot(node->right.get(), pieces);
}

std::size_t PieceTable::lineStart(std::size_t line) const {
    if (line == 0 || !root_) {
        return 0;
    }

    std::size_t remaining = line;
    std::size_t base = 0;
    const Node* node = root_.get();

    while (node) {
        const std::size_t leftNewlines = newlinesOf(node->left.get());

        if (remaining <= leftNewlines) {
            node = node->left.get();
            continue;
        }
        remaining -= leftNewlines;

        if (remaining <= node->pieceNewlines) {
            const std::string& source = sourceFor(node);
            const std::size_t index = newlineOffsetInPiece(node, source, remaining);
            return base + bytesOf(node->left.get()) + index + 1;
        }
        remaining -= node->pieceNewlines;

        base += bytesOf(node->left.get()) + node->length;
        node = node->right.get();
    }

    // Past the last newline: clamp to the end of the document, which is the
    // start of the final (possibly empty) line.
    return size();
}

std::size_t PieceTable::lineOfOffset(std::size_t offset) const {
    if (offset >= size()) {
        return newlineCount();
    }

    std::size_t line = 0;
    std::size_t base = 0;
    const Node* node = root_.get();

    while (node) {
        const std::size_t leftBytes = bytesOf(node->left.get());
        const std::size_t nodeStart = base + leftBytes;

        if (offset < nodeStart) {
            node = node->left.get();
            continue;
        }

        if (offset < nodeStart + node->length) {
            const std::size_t local = offset - nodeStart;
            const std::string& source = sourceFor(node);
            return line + newlinesOf(node->left.get()) +
                   countNewlines(std::string_view(source).substr(node->start, local), local);
        }

        line += newlinesOf(node->left.get()) + node->pieceNewlines;
        base = nodeStart + node->length;
        node = node->right.get();
    }

    return line;
}

} // namespace hexal
