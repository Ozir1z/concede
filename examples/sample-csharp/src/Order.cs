namespace Sample;

/// <summary>
/// A customer order. An order starts as a draft and can be submitted once it
/// has at least one line and a positive total.
/// </summary>
public sealed class Order
{
    private readonly List<OrderLine> _lines = new();

    public Order(string customerId)
    {
        if (string.IsNullOrWhiteSpace(customerId))
            throw new ArgumentException("customerId is required", nameof(customerId));

        CustomerId = customerId;
    }

    public string CustomerId { get; }

    public OrderStatus Status { get; private set; } = OrderStatus.Draft;

    public IReadOnlyList<OrderLine> Lines => _lines;

    public decimal Total => _lines.Sum(line => line.UnitPrice * line.Quantity);

    public void AddLine(string sku, int quantity, decimal unitPrice)
    {
        if (Status != OrderStatus.Draft)
            throw new InvalidOperationException("Only draft orders can be changed.");

        _lines.Add(new OrderLine(sku, quantity, unitPrice));
    }

    /// <summary>
    /// Move the order from draft to submitted. Throws if the order is empty or
    /// already submitted.
    /// </summary>
    public void Submit()
    {
        if (Status != OrderStatus.Draft)
            throw new InvalidOperationException("Order has already been submitted.");

        if (_lines.Count == 0)
            throw new InvalidOperationException("Cannot submit an empty order.");

        if (Total <= 0)
            throw new InvalidOperationException("Order total must be positive.");

        Status = OrderStatus.Submitted;
    }
}

public enum OrderStatus
{
    Draft,
    Submitted,
}

public readonly record struct OrderLine(string Sku, int Quantity, decimal UnitPrice);
