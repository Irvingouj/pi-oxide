interface ErrorFallbackProps {
	title?: string;
}

export default function ErrorFallback({
	title = "Something went wrong",
}: ErrorFallbackProps) {
	return (
		<div className="p-4 bg-danger/10 border border-danger rounded-2xl text-danger text-sm">
			<p className="font-semibold">{title}</p>
		</div>
	);
}
